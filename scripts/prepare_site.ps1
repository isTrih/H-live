$ErrorActionPreference = 'Stop'
$releaseRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
& node (Join-Path $PSScriptRoot 'site_bundle.mjs') $releaseRoot
if ($LASTEXITCODE -ne 0) { throw 'Release website preparation failed.' }
