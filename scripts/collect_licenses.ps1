# Collect exact Rust/JS/Font notices for the pure Rust distribution.
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$destination = Join-Path $root 'packaging/licenses'
$resolvedDestination = [IO.Path]::GetFullPath($destination)
if ($resolvedDestination -ne [IO.Path]::GetFullPath((Join-Path $root 'packaging/licenses'))) { throw 'License target escaped project' }
if (Test-Path -LiteralPath $resolvedDestination) { Remove-Item -LiteralPath $resolvedDestination -Recurse -Force }
New-Item -ItemType Directory -Path $resolvedDestination -Force | Out-Null
$notices = [Collections.Generic.List[string]]::new()
$version = (Get-Content -LiteralPath (Join-Path $root 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json).version
$notices.Add("H-live / 氢看直播 $version - Rust, JavaScript and font dependencies")
$notices.Add('Upstream notices: LICENSE and licenses/upstream. Dependency texts: licenses/.')
$seen = @{}
foreach ($manifest in @('src-tauri/Cargo.toml','crates/Cargo.toml')) {
    $selection = if ($manifest -like 'crates/*') { @('-p','h-live-runtime') } else { @() }
    $lines = & cargo tree --manifest-path (Join-Path $root $manifest) --target x86_64-pc-windows-msvc --locked --offline --edges normal,build --prefix none --format '{p}' @selection
    if ($LASTEXITCODE -ne 0) { throw 'Cargo dependency tree failed' }
    $wanted = @{}
    foreach ($line in $lines) { if ($line -match '^(\S+) v(\S+)') { $wanted[($Matches[1] + '-' + $Matches[2])] = $true } }
    $metadataText = & cargo metadata --manifest-path (Join-Path $root $manifest) --format-version 1 --locked --offline --filter-platform x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
    $metadata = ($metadataText -join "`n") | ConvertFrom-Json
    foreach ($package in $metadata.packages) {
        $key = $package.name + '-' + $package.version
        if (-not $package.source -or -not $wanted.ContainsKey($key) -or $seen.ContainsKey($key)) { continue }
        $source = Split-Path -Parent $package.manifest_path
        $files = @(Get-ChildItem -LiteralPath $source -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|AUTHORS|COPYRIGHT)' })
        if ($package.license_file) { $specific = Join-Path $source $package.license_file; if (Test-Path -LiteralPath $specific -PathType Leaf) { $files += Get-Item -LiteralPath $specific } }
        if (-not $files.Count) { $files = @(Get-ChildItem -LiteralPath $source -File -Recurse | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING)' } | Select-Object -First 8) }
        if (-not $files.Count) { $extra = Join-Path $root "packaging/extra-licenses/$key"; if (Test-Path -LiteralPath $extra) { $files = @(Get-ChildItem -LiteralPath $extra -File) } }
        if (-not $files.Count) { throw "License review required: $key" }
        $target = Join-Path $destination "rust/$key"
        New-Item -ItemType Directory -Path $target -Force | Out-Null
        $index = 0
        foreach ($file in ($files | Sort-Object FullName -Unique)) { Copy-Item -LiteralPath $file.FullName -Destination (Join-Path $target ($index.ToString() + '-' + $file.Name)); $index++ }
        $notices.Add("$($package.name) $($package.version): $($package.license)")
        $seen[$key] = $true
    }
}
foreach ($name in @('@tauri-apps/api','@tauri-apps/plugin-updater','@tauri-apps/cli','vite')) {
    $source = Join-Path $root "node_modules/$name"
    $package = Get-Content -LiteralPath (Join-Path $source 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $target = Join-Path $destination ('js/' + $name.Replace('/','-').Replace('@',''))
    New-Item -ItemType Directory -Path $target -Force | Out-Null
    $files = @(Get-ChildItem -LiteralPath $source -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' })
    if (-not $files.Count) { throw "Missing JS license: $name" }
    foreach ($file in $files) { Copy-Item -LiteralPath $file.FullName -Destination $target }
    $notices.Add("$name $($package.version): $($package.license)")
}
$fontManifest = Get-Content -LiteralPath (Join-Path $root 'src/fonts/font-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
foreach ($asset in $fontManifest.assets) {
    if ((Get-FileHash -LiteralPath (Join-Path $root $asset.file) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $asset.sha256) { throw "Font/license bytes changed: $($asset.file)" }
}
New-Item -ItemType Directory -Path (Join-Path $destination 'fonts') -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $root 'src/fonts/licenses') -Destination (Join-Path $destination 'fonts/licenses') -Recurse
Copy-Item -LiteralPath (Join-Path $root 'src/fonts/font-manifest.json') -Destination (Join-Path $destination 'fonts/font-manifest.json')
Copy-Item -LiteralPath (Join-Path $root 'licenses/upstream') -Destination (Join-Path $destination 'upstream') -Recurse
Copy-Item -LiteralPath (Join-Path $root 'docs/research/licenses/douyinlive-proto-MIT.txt') -Destination (Join-Path $destination 'douyinlive-proto-MIT.txt')
Copy-Item -LiteralPath (Join-Path $root 'packaging/extra-licenses/nsis-3.11') -Destination (Join-Path $destination 'nsis-3.11') -Recurse
$notices.Add('Unmodified fonts: Douyin Sans Bold and ChillRoundM, SIL OFL 1.1. Font copyrights/licenses are separate from the project license.')
$notices.Add('NSIS and WebView2 retain their own notices. This distribution has no Python, PyO3, V8 or Node runtime.')
[IO.File]::WriteAllText((Join-Path $root 'THIRD_PARTY_DESKTOP_NOTICES.txt'), (($notices -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText((Join-Path $root 'THIRD_PARTY_NOTICES.txt'), "H-live $version uses a Rust collector and Tauri WebView. See THIRD_PARTY_DESKTOP_NOTICES.txt, LICENSE, DISCLAIMER.md and licenses/.`n", [Text.UTF8Encoding]::new($false))
Write-Output "Collected $($seen.Count) Rust dependency license groups plus JavaScript, fonts and upstream notices."
