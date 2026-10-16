# Build the required CPython extension using the same local Rust toolchain.
param([string]$Python = '')
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $Python) { $Python = Join-Path $projectRoot '.venv\Scripts\python.exe' }
if (-not (Test-Path -LiteralPath $Python)) { throw 'The configured CPython interpreter does not exist.' }
$oldPyO3Python = $env:PYO3_PYTHON
$oldPythonPath = $env:PYTHONPATH
Push-Location $projectRoot
try {
    . (Join-Path $PSScriptRoot 'rust-env.ps1')
    $env:PYO3_PYTHON = (Resolve-Path -LiteralPath $Python).Path
    & cargo build --manifest-path crates/Cargo.toml --target-dir src-tauri/target --release --locked --offline -p qingkan-native
    if ($LASTEXITCODE -ne 0) { throw 'Native archive/export/protocol extension build failed.' }
    $collector = Join-Path $projectRoot 'python\collector'
    Copy-Item -LiteralPath (Join-Path $projectRoot 'src-tauri\target\release\qingkan_native.dll') -Destination (Join-Path $collector 'qingkan_native.pyd') -Force
    $env:PYTHONPATH = $collector
    & $Python -c 'import json, pathlib, qingkan_native; expected=json.loads(pathlib.Path("package.json").read_text(encoding="utf-8"))["version"]; assert qingkan_native.__version__ == expected, (qingkan_native.__version__, expected); print("Native extension:", qingkan_native.__version__)'
    if ($LASTEXITCODE -ne 0) { throw 'Native extension import/version validation failed.' }
} finally {
    $env:PYO3_PYTHON = $oldPyO3Python
    $env:PYTHONPATH = $oldPythonPath
    Pop-Location
}
