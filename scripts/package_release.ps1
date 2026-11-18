param([string]$SigningKey = '')
$ErrorActionPreference = 'Stop'
$releaseRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$releaseVersion = (Get-Content -LiteralPath (Join-Path $releaseRoot 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json).version
$site = Get-Content -LiteralPath (Join-Path $releaseRoot 'release-site/publishing.json') -Raw | ConvertFrom-Json
$releaseBase = $site.url.TrimEnd('/') + $site.base_path.TrimEnd('/') + '/'
if (([uri]$releaseBase).Scheme -ne 'https') { throw 'Release base must use HTTPS.' }
$oldSigningKey = $env:TAURI_SIGNING_PRIVATE_KEY
$oldSigningPassword = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
Push-Location $releaseRoot
try {
    . (Join-Path $PSScriptRoot 'rust-env.ps1')
    & (Join-Path $PSScriptRoot 'stage_release_resources.ps1')
    # Keep the directory distribution's documents identical to the installer.
    Get-ChildItem -LiteralPath 'src-tauri\bundle-docs' | Copy-Item -Destination 'dist\DouyinLiveMonitor' -Recurse -Force
    if (-not $SigningKey) { $SigningKey = Join-Path $releaseRoot '.release-secrets\qingkan-updater.key' }
    if (-not (Test-Path -LiteralPath $SigningKey)) { throw 'Release signing key is missing. Keep the original private key securely; do not regenerate it for an update.' }
    # Tauri accepts a path here. Never put the key contents into a command line or log.
    $env:TAURI_SIGNING_PRIVATE_KEY = (Resolve-Path -LiteralPath $SigningKey).Path
    if ($null -eq $oldSigningPassword) { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '' }
    & .\node_modules\.bin\tauri.cmd bundle --bundles nsis --ci
    if ($LASTEXITCODE -ne 0) { throw 'NSIS bundle/signature generation failed.' }
    $productName = (Get-Content -LiteralPath 'src-tauri\tauri.conf.json' -Raw -Encoding UTF8 | ConvertFrom-Json).productName
    $source = Join-Path $releaseRoot "src-tauri\target\release\bundle\nsis\${productName}_${releaseVersion}_x64-setup.exe"
    if (-not (Test-Path -LiteralPath $source)) { throw 'Expected the exact configured product/version x64 NSIS installer.' }
    if (-not (Test-Path -LiteralPath ($source + '.sig'))) { throw 'The signed update artifact is missing its .sig file.' }
    $fileName = "QingkanLive-${releaseVersion}-windows-x64-setup.exe"
    $installerDir = Join-Path $releaseRoot 'dist\installer'
    $cdnRoot = Join-Path $releaseRoot 'dist\cdn\qingkan-live'
    $cdnVersion = Join-Path $cdnRoot $releaseVersion
    New-Item -ItemType Directory -Path $installerDir, $cdnVersion -Force | Out-Null
    $setup = Join-Path $installerDir $fileName
    Copy-Item -LiteralPath $source -Destination $setup -Force
    Copy-Item -LiteralPath ($source + '.sig') -Destination ($setup + '.sig') -Force
    Copy-Item -LiteralPath $setup, ($setup + '.sig') -Destination $cdnVersion -Force
    $hash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
    ($hash + '  ' + $fileName) | Set-Content -LiteralPath ($setup + '.sha256') -Encoding ascii
    Copy-Item -LiteralPath ($setup + '.sha256') -Destination $cdnVersion -Force
    # NSIS bundling marks the main EXE with its bundle type for the updater.
    Copy-Item -LiteralPath 'src-tauri\target\release\DouyinLiveMonitor.exe' -Destination 'dist\DouyinLiveMonitor\DouyinLiveMonitor.exe' -Force
    $notesPath = Join-Path $releaseRoot "release-notes\${releaseVersion}.md"
    if (-not (Test-Path -LiteralPath $notesPath)) { throw 'Release notes are required for a publishable update.' }
    $releaseNotes = (Get-Content -LiteralPath $notesPath -Raw -Encoding UTF8).Replace("`r`n", "`n").Replace("`r", "`n")
    $latest = [ordered]@{
        version = $releaseVersion
        notes = $releaseNotes
        pub_date = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
        platforms = @{'windows-x86_64' = @{
            signature = (Get-Content -LiteralPath ($setup + '.sig') -Raw -Encoding UTF8).Trim()
            url = "${releaseBase}${releaseVersion}/${fileName}"
        }}
    }
    [IO.File]::WriteAllText((Join-Path $cdnRoot 'latest.json'), ($latest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    # Retain this version's first declared date before the publication payload is frozen.
    & node scripts/release_notes_data.mjs --record-manifest $releaseRoot (Join-Path $cdnRoot 'latest.json')
    if ($LASTEXITCODE -ne 0) { throw 'Cannot retain release date metadata.' }
    Copy-Item -LiteralPath 'docs\release\CDN_RELEASE.md' -Destination (Join-Path $cdnRoot 'UPLOAD-INSTRUCTIONS.md') -Force
    Write-Output $setup
    Write-Output "SHA256 $hash"
} finally {
    $env:TAURI_SIGNING_PRIVATE_KEY = $oldSigningKey
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $oldSigningPassword
    Pop-Location
}
