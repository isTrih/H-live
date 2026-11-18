# A fixed shared LGPL build; never resolve executables from the user's PATH.
[CmdletBinding()]
param([switch]$Offline)
$ErrorActionPreference = 'Stop'
$mediaProject = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$mediaManifestPath = Join-Path $mediaProject 'packaging/media-tools.json'
$mediaManifest = Get-Content -LiteralPath $mediaManifestPath -Raw | ConvertFrom-Json
$mediaCache = Join-Path $mediaProject 'packaging/tools/downloads'
$mediaArchive = Join-Path $mediaCache 'ffmpeg-n8.1.3-24-lgpl-shared.zip'
$mediaExtract = Join-Path $mediaProject 'packaging/tools/ffmpeg-pinned'
$mediaSource = Join-Path $mediaExtract $mediaManifest.directory
$mediaDestination = Join-Path $mediaProject 'packaging/tools/media'
New-Item -ItemType Directory -Path $mediaCache -Force | Out-Null
if (-not (Test-Path -LiteralPath $mediaArchive -PathType Leaf)) {
    if ($Offline) { throw 'Pinned media tools are not cached. Run scripts/prepare_media_tools.ps1 once with network access.' }
    Invoke-WebRequest -Uri $mediaManifest.url -OutFile $mediaArchive
}
if ((Get-FileHash -LiteralPath $mediaArchive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $mediaManifest.sha256) {
    throw 'Pinned FFmpeg archive checksum mismatch; retained for inspection.'
}
if (-not (Test-Path -LiteralPath (Join-Path $mediaSource 'bin/ffprobe.exe'))) {
    Expand-Archive -LiteralPath $mediaArchive -DestinationPath $mediaExtract -Force
}
New-Item -ItemType Directory -Path $mediaDestination -Force | Out-Null
foreach ($mediaTool in @('ffmpeg.exe', 'ffprobe.exe')) {
    Copy-Item -LiteralPath (Join-Path $mediaSource "bin/$mediaTool") -Destination $mediaDestination -Force
}
Get-ChildItem -LiteralPath (Join-Path $mediaSource 'bin') -Filter '*.dll' -File | Copy-Item -Destination $mediaDestination -Force
Copy-Item -LiteralPath (Join-Path $mediaSource 'LICENSE.txt') -Destination (Join-Path $mediaDestination 'FFmpeg-LICENSE.txt') -Force
Copy-Item -LiteralPath $mediaManifestPath -Destination (Join-Path $mediaDestination 'source.json') -Force
$mediaBuildInfo = & (Join-Path $mediaDestination 'ffmpeg.exe') -hide_banner -version 2>&1
if ($LASTEXITCODE -ne 0 -or ($mediaBuildInfo -join "`n") -match '--enable-gpl') { throw 'Media tools are unusable or unexpectedly GPL-enabled.' }
$mediaBuildInfo | Set-Content -LiteralPath (Join-Path $mediaDestination 'build-info.txt') -Encoding utf8
$mpvManifestPath = Join-Path $mediaProject 'packaging/native-player.json'
$mpvManifest = Get-Content -LiteralPath $mpvManifestPath -Raw | ConvertFrom-Json
$mpvArchive = Join-Path $mediaCache 'libmpv-0fae46954-lgpl.zip'
if (-not (Test-Path -LiteralPath $mpvArchive -PathType Leaf)) {
    if ($Offline) { throw 'Pinned native player is not cached. Run scripts/prepare_media_tools.ps1 once with network access.' }
    Invoke-WebRequest -Uri $mpvManifest.url -OutFile $mpvArchive
}
if ((Get-FileHash -LiteralPath $mpvArchive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $mpvManifest.sha256) { throw 'Pinned libmpv archive checksum mismatch.' }
$mpvSource = Join-Path $mediaProject 'packaging/tools/mpv-pinned'
if (-not (Test-Path -LiteralPath (Join-Path $mpvSource 'libmpv-2.dll'))) { Expand-Archive -LiteralPath $mpvArchive -DestinationPath $mpvSource -Force }
Copy-Item -LiteralPath (Join-Path $mpvSource 'libmpv-2.dll') -Destination $mediaDestination -Force
Copy-Item -LiteralPath (Join-Path $mpvSource 'LICENSE.LGPL') -Destination (Join-Path $mediaDestination 'libmpv-LICENSE.txt') -Force
Copy-Item -LiteralPath $mpvManifestPath -Destination (Join-Path $mediaDestination 'native-player-source.json') -Force
Write-Host "Pinned media tools ready: $($mediaManifest.version)"
