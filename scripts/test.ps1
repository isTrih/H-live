param([ValidateSet('all','frontend','rust')][string]$Suite='all')
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    if ($Suite -in @('all','frontend')) {
        $files = @(Get-ChildItem -LiteralPath src -Filter '*.test.js' -File -Recurse | Select-Object -ExpandProperty FullName)
        & node --test @files
        if ($LASTEXITCODE -ne 0) { throw 'Frontend tests failed' }
    }
    if ($Suite -in @('all','rust')) {
        . ./scripts/rust-env.ps1
        & cargo test --manifest-path crates/Cargo.toml --target-dir src-tauri/target --locked --offline -p qingkan-core -p h-live-runtime --features h-live-runtime/test-support -- --test-threads=4
        if ($LASTEXITCODE -ne 0) { throw 'Rust runtime/core tests failed' }
        & cargo test --manifest-path src-tauri/Cargo.toml --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'Desktop tests failed' }
    }
} finally { Pop-Location }
