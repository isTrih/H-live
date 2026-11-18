# Website-only revision of an already frozen release. No version bump or build.
$ErrorActionPreference = 'Stop'
$releaseRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$releaseLock = $null
try {
    $privateDirectory = Join-Path $releaseRoot '.release-secrets'
    New-Item -ItemType Directory -Path $privateDirectory -Force | Out-Null
    try { $releaseLock = [IO.File]::Open((Join-Path $privateDirectory 'release.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) }
    catch { throw 'Another release process holds the release lock. Wait for it to finish before refreshing the website.' }
    & node (Join-Path $PSScriptRoot 'release_version.mjs') refresh-site $releaseRoot
    if ($LASTEXITCODE -ne 0) { throw 'Website revision failed. The retained release and private snapshot must be kept for recovery.' }
} finally {
    if ($releaseLock) { $releaseLock.Dispose() }
}
