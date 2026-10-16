# Test current product modules. Legacy Tk tests are intentionally separate.
param(
    [ValidateSet('all', 'python', 'frontend', 'rust')][string]$Suite = 'all',
    [string]$Python = '',
    [switch]$SkipNativeBuild
)
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $Python) { $Python = Join-Path $projectRoot '.venv\Scripts\python.exe' }
$oldPythonPath = $env:PYTHONPATH
Push-Location $projectRoot
try {
    # Root remains a transitional path for reference adapters only. Collector wins.
    $env:PYTHONPATH = (Join-Path $projectRoot 'python\collector') + [IO.Path]::PathSeparator + $projectRoot
    if ($Suite -in @('all', 'python')) {
        if (-not $SkipNativeBuild) { & (Join-Path $PSScriptRoot 'build_native.ps1') -Python $Python }
        & $Python -m unittest discover -s tests -v
        if ($LASTEXITCODE -ne 0) { throw 'Collector tests failed.' }
        & $Python -m unittest discover -s backend_tests -v
        if ($LASTEXITCODE -ne 0) { throw 'Backend contract tests failed.' }
        & $Python -m unittest discover -s native_tests -v
        if ($LASTEXITCODE -ne 0) { throw 'Native adapter and protocol differential tests failed.' }
    }
    if ($Suite -in @('all', 'frontend')) {
        $testFiles = @(Get-ChildItem -LiteralPath (Join-Path $projectRoot 'src') -Filter '*.test.js' -File -Recurse | Select-Object -ExpandProperty FullName)
        & node --test @testFiles
        if ($LASTEXITCODE -ne 0) { throw 'Frontend tests failed.' }
    }
    if ($Suite -in @('all', 'rust')) {
        . (Join-Path $PSScriptRoot 'rust-env.ps1')
        & cargo test --manifest-path src-tauri/Cargo.toml --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'Desktop shell tests failed.' }
        & cargo test --manifest-path crates/Cargo.toml --target-dir src-tauri/target --locked --offline -p qingkan-core
        if ($LASTEXITCODE -ne 0) { throw 'Native core tests failed.' }
    }
} finally {
    $env:PYTHONPATH = $oldPythonPath
    Pop-Location
}
