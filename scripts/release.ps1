param([string]$SigningKey = '', [string]$NotesPath = '', [ValidateSet('patch','minor','major')][string]$Bump = 'patch', [switch]$SkipBuild, [switch]$RunTests)
$ErrorActionPreference = 'Stop'
$releaseRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$releaseLock = $null
Push-Location $releaseRoot
try {
    $privateDirectory = Join-Path $releaseRoot '.release-secrets'
    New-Item -ItemType Directory -Path $privateDirectory -Force | Out-Null
    try { $releaseLock = [IO.File]::Open((Join-Path $privateDirectory 'release.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) }
    catch { throw 'Another release process holds the release lock. Wait for it to finish before retrying.' }
    $versionArguments = @('scripts/release_version.mjs', 'prepare', $releaseRoot)
    if ($NotesPath) { $versionArguments += $NotesPath }
    $versionArguments += "--bump=$Bump"
    $prepared = & node @versionArguments
    if ($LASTEXITCODE -ne 0) { throw 'Release version preparation failed.' }
    $release = $prepared | ConvertFrom-Json
    Write-Host "Release $($release.version) ($($release.phase), resume=$($release.reused))"
    if (-not $SigningKey) {
        $localConfig = Join-Path $releaseRoot '.release-secrets/release-local.json'
        if (Test-Path -LiteralPath $localConfig) { $SigningKey = (Get-Content -LiteralPath $localConfig -Raw | ConvertFrom-Json).signing_key_path }
        elseif ($env:QINGKAN_SIGNING_KEY_PATH) { $SigningKey = $env:QINGKAN_SIGNING_KEY_PATH }
    }
    if ($release.phase -ne 'publishing') {
        if ($RunTests) {
            & ./scripts/test.ps1 -Suite frontend
            & ./scripts/test.ps1 -Suite rust
        }
        if (-not $SkipBuild) {
            & ./scripts/build.ps1 -SkipInstaller
        } else {
            $smokePath = Join-Path $releaseRoot "build/smoke-report-$($release.version).json"
            if (-not (Test-Path -LiteralPath $smokePath)) { throw '-SkipBuild requires a completed build for this exact prepared version. Rerun without -SkipBuild.' }
            $smoke = Get-Content -LiteralPath $smokePath -Raw | ConvertFrom-Json
            if (-not $smoke.ok -or $smoke.version -ne $release.version -or $smoke.checks.native_version -ne $release.version -or $smoke.checks.runtime_version -ne $release.version) { throw 'The retained build is not the prepared release version.' }
        }
        & ./scripts/package_release.ps1 -SigningKey $SigningKey
        & ./scripts/verify_release.ps1
        & ./scripts/prepare_site.ps1
        & node ./scripts/release_version.mjs publishing $releaseRoot
        if ($LASTEXITCODE -ne 0) { throw 'Cannot retain the prepared publication transaction.' }
    }
    # Retried publication keeps the same files and official Sites request IDs.
    & node ./scripts/publish_sites.mjs
    if ($LASTEXITCODE -eq 2) {
        Write-Host '完整安装包已准备。请在 Sites 原站点上传 dist/sites/qingkan-live 文件夹；本次版本保持待发布，确认上线并验证后再完成发布回执。'
        return
    }
    if ($LASTEXITCODE -ne 0) { throw 'Sites publication failed; inspect retained deployment before retrying.' }
    & node ./scripts/release_version.mjs complete $releaseRoot
    if ($LASTEXITCODE -ne 0) { throw 'The publication completed but its local receipt could not be committed. Retain the release state and retry.' }
} finally {
    if ($releaseLock) { $releaseLock.Dispose() }
    Pop-Location
}
