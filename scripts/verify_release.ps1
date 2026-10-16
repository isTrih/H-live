param([switch]$SkipVerifierBuild)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    . ./scripts/rust-env.ps1
    if (-not $SkipVerifierBuild) {
        & cargo build --manifest-path packaging/signature-check/Cargo.toml --target-dir build/signature-check --release --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'Public signature verifier build failed' }
    }
    $config = Get-Content src-tauri/tauri.conf.json -Raw -Encoding UTF8 | ConvertFrom-Json
    $package = Get-Content package.json -Raw -Encoding UTF8 | ConvertFrom-Json
    $version = $package.version
    $cdn = Join-Path $root 'dist/cdn/qingkan-live'
    $manifest = Get-Content (Join-Path $cdn 'latest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $filename = "QingkanLive-$version-windows-x64-setup.exe"
    $installer = Join-Path $cdn "$version/$filename"
    $signature = $installer + '.sig'
    $platform = $manifest.platforms.'windows-x86_64'
    $base = 'https://assets.checkpoint321.com/lilith/qingkan-live/'
    $hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
    $checks = [ordered]@{
        versions_match = ($config.version -eq $version -and $manifest.version -eq $version -and
            (Get-Content src-tauri/Cargo.toml -Raw) -match ('(?m)^version = "' + [regex]::Escape($version) + '"') -and
            (Get-Content crates/Cargo.toml -Raw) -match ('(?m)^version = "' + [regex]::Escape($version) + '"'))
        endpoint_matches = (@($config.plugins.updater.endpoints).Count -eq 1 -and $config.plugins.updater.endpoints[0] -eq $base + 'latest.json')
        download_matches = ($platform.url -eq "$base$version/$filename")
        manifest_signature_matches = ($platform.signature -ceq (Get-Content $signature -Raw -Encoding UTF8).Trim())
        notes_match = ($manifest.notes.Trim() -ceq (Get-Content "release-notes/$version.md" -Raw -Encoding UTF8).Trim())
        sha256_matches = ((Get-Content ($installer + '.sha256') -Raw).Trim() -ceq "$hash  $filename")
        installer_copy_matches = ((Get-FileHash "dist/installer/$filename").Hash.ToLowerInvariant() -eq $hash)
        no_engine_resource = (-not $config.bundle.resources.PSObject.Properties['engine/'])
    }
    $publicKey = Join-Path $root 'build/updater-public-key.pub'
    [IO.File]::WriteAllText($publicKey, $config.plugins.updater.pubkey, [Text.Encoding]::ASCII)
    $verification = & ./build/signature-check/release/qingkan-signature-check.exe $publicKey $signature $installer
    if ($LASTEXITCODE -ne 0) { throw 'Signature verification or tamper rejection failed' }
    $verification = $verification | ConvertFrom-Json
    $checks.signature_valid = $verification.signature_valid
    $checks.tampered_artifact_rejected = $verification.tampered_rejected
    $checks.verifier_hash_matches = ($verification.sha256 -eq $hash)
    $payload = @(Get-ChildItem dist/DouyinLiveMonitor -File -Recurse)
    $forbidden = @($payload | Where-Object { $_.FullName -match '(?i)[\\/](python[^\\/]*|node\.exe|qingkan_native\.pyd|v8[^\\/]*|mini_racer[^\\/]*|engine)[\\/.]' -or $_.Extension -in @('.pyd','.py','.pyc') })
    $checks.no_sidecar_payload = ($forbidden.Count -eq 0 -and @(Get-ChildItem dist/DouyinLiveMonitor -Filter '*.exe').Count -eq 1)
    $checks.binary_version_matches = ((Get-Item dist/DouyinLiveMonitor/DouyinLiveMonitor.exe).VersionInfo.FileVersion -eq $version)
    $report = [ordered]@{ ok=(@($checks.Values | Where-Object { -not $_ }).Count -eq 0); version=$version; sha256=$hash; bytes=(Get-Item $installer).Length; checks=$checks; signature_verification=$verification; payload_files=$payload.Count; private_key_read=$false }
    $report | ConvertTo-Json -Depth 8 | Set-Content "build/release-verification-$version.json" -Encoding utf8
    $report | ConvertTo-Json -Depth 8
    if (-not $report.ok) { throw 'Release validation did not pass every check' }
} finally { Pop-Location }
