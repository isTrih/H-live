# Stage only public release resources, preserving the repository's relative links.
$ErrorActionPreference = 'Stop'
$releaseRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$docTarget = Join-Path $releaseRoot 'src-tauri\bundle-docs'
$licenseTarget = Join-Path $releaseRoot 'src-tauri\bundle-licenses'
foreach ($generated in @($docTarget, $licenseTarget)) {
    $resolved = [IO.Path]::GetFullPath($generated)
    if (-not $resolved.StartsWith($releaseRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Resource staging escaped workspace.' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
    New-Item -ItemType Directory -Path $resolved -Force | Out-Null
}
$manifest = Get-Content -LiteralPath (Join-Path $releaseRoot 'packaging\release-resources.json') -Raw -Encoding UTF8 | ConvertFrom-Json
foreach ($document in $manifest.files) {
    $source = [IO.Path]::GetFullPath((Join-Path $releaseRoot $document))
    $destination = [IO.Path]::GetFullPath((Join-Path $docTarget $document))
    if (-not $source.StartsWith($releaseRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or
        -not $destination.StartsWith($docTarget + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid release resource path.' }
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Required release document is missing: $document" }
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item -LiteralPath $source -Destination $destination -Force
}
foreach ($directory in $manifest.directories) {
    $source = [IO.Path]::GetFullPath((Join-Path $releaseRoot $directory))
    $destination = [IO.Path]::GetFullPath((Join-Path $docTarget $directory))
    if ($directory -ne 'release-notes' -or -not $source.StartsWith($releaseRoot + '\') -or -not $destination.StartsWith($docTarget + '\')) { throw 'Unexpected release document directory.' }
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    Get-ChildItem -LiteralPath $source -File -Filter '*.md' | Copy-Item -Destination $destination -Force
}
Get-ChildItem -LiteralPath (Join-Path $releaseRoot 'packaging\licenses') | Copy-Item -Destination $licenseTarget -Recurse -Force
