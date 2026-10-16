param([switch]$SkipFixtureBuild, [string]$BaselineInstaller, [string]$BaselineVersion)
$ErrorActionPreference = 'Stop'
$testRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$testVersion = (Get-Content (Join-Path $testRoot 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json).version
$fixtureName = 'QingkanWorkspaceTest'
$testDir = Join-Path $testRoot "build\nsis-install-check-$testVersion"
$installDir = Join-Path $testDir 'app'
$dataDir = Join-Path $testDir 'data'
$legacyDir = Join-Path $testDir 'legacy-guard'
$registry = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$fixtureName"
$oldInnoKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{97F3675A-88D2-426D-90DA-CB4B8F831731}_is1'
if (Test-Path -LiteralPath $registry) { throw 'Previous workspace NSIS test registration exists; inspect before repeating.' }
foreach ($path in @($testDir, $installDir, $dataDir, $legacyDir)) {
    if (-not [IO.Path]::GetFullPath($path).StartsWith($testRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Test path escaped workspace.' }
    New-Item -ItemType Directory -Path $path -Force | Out-Null
}
function Snapshot-OldInstall {
    if (Test-Path -LiteralPath $oldInnoKey) {
        Get-ItemProperty -LiteralPath $oldInnoKey | Select-Object DisplayVersion, InstallLocation, UninstallString | ConvertTo-Json -Compress
    } else { 'absent' }
}
function Run-Installer([string]$Exe, [string]$Arguments) {
    $process = Start-Process -FilePath $Exe -ArgumentList $Arguments -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(120000)) { throw 'Workspace installer timed out; inspect its own process without closing user apps.' }
    $process.Refresh()
    return $process.ExitCode
}
function Test-InstalledRuntime([string]$ReportName, [string]$ExpectedVersion) {
    $oldDataDir = $env:DOUYIN_MONITOR_HOME
    try {
        $env:DOUYIN_MONITOR_HOME = $dataDir
        $selfReport = Join-Path $testDir $ReportName
        $self = Start-Process -FilePath (Join-Path $installDir "$fixtureName.exe") -ArgumentList @('--self-test', ('"'+$selfReport+'"')) -WindowStyle Hidden -PassThru
        if (-not $self.WaitForExit(60000)) { throw 'Installed self-test timed out.' }
        $self.Refresh()
        $result = Get-Content -LiteralPath $selfReport -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($self.ExitCode -ne 0 -or -not $result.ok -or $result.version -ne $ExpectedVersion) { throw 'Installed self-test or version check failed.' }
    } finally { $env:DOUYIN_MONITOR_HOME = $oldDataDir }
}
$oldSnapshot = Snapshot-OldInstall
if ($BaselineInstaller) {
    $BaselineInstaller = (Resolve-Path -LiteralPath $BaselineInstaller).Path
    if ($BaselineVersion -notmatch '^\d+\.\d+\.\d+$') { throw 'Provide the exact semantic version of the preserved workspace fixture.' }
    if (-not $BaselineInstaller.StartsWith($testRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($BaselineInstaller) -ne "QingkanWorkspaceTest_${BaselineVersion}_x64-setup.exe") {
        throw 'Only a preserved, independently named workspace fixture can be used as the upgrade baseline.'
    }
}
$previewPids = @(Get-Process DouyinLiveMonitor -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$testRoot\build\dev-preview\*" } | Select-Object -ExpandProperty Id)
$report = [ordered]@{ ok=$false; scope='Isolated NSIS fixture: distinct installer registry identity and binary filename; release payload bytes shared'; version=$testVersion; install_dir=$installDir; user_inno_untouched=$false; user_preview_untouched=$false }
Push-Location $testRoot
try {
    . (Join-Path $testRoot 'scripts\rust-env.ps1')
    & (Join-Path $testRoot 'scripts\stage_release_resources.ps1')
    $fixtureExe = Join-Path $testRoot "src-tauri\target\release\$fixtureName.exe"
    Copy-Item -LiteralPath 'src-tauri\target\release\DouyinLiveMonitor.exe' -Destination $fixtureExe -Force
    $configuration = @{
        productName=$fixtureName; identifier='com.douyinlivemonitor.packagingtest'; mainBinaryName=$fixtureName
        bundle=@{createUpdaterArtifacts=$false; windows=@{nsis=@{startMenuFolder=$fixtureName}}}
    }
    $configPath = Join-Path $testDir 'config.json'
    [IO.File]::WriteAllText($configPath, ($configuration | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    if (-not $SkipFixtureBuild) {
        & .\node_modules\.bin\tauri.cmd bundle --bundles nsis --ci --config $configPath
        if ($LASTEXITCODE -ne 0) { throw 'Workspace NSIS fixture compilation failed.' }
    }
    $setup = Join-Path $testRoot "src-tauri\target\release\bundle\nsis\${fixtureName}_${testVersion}_x64-setup.exe"
    $report.fixture_installer_sha256 = (Get-FileHash -LiteralPath $setup).Hash.ToLowerInvariant()
    'This is a harmless guard fixture, not an executable.' | Set-Content -LiteralPath (Join-Path $legacyDir 'unins000.exe')
    $guardExit = Run-Installer $setup "/S /NS /D=$legacyDir"
    if ($guardExit -eq 0 -or (Test-Path -LiteralPath (Join-Path $legacyDir "$fixtureName.exe"))) { throw 'Legacy installer directory guard failed.' }
    $report.legacy_directory_guard_exit = $guardExit
    'retain this data' | Set-Content -LiteralPath (Join-Path $dataDir 'preserve.txt')
    if ($BaselineInstaller) {
        $baselineExit = Run-Installer $BaselineInstaller "/S /NS /D=$installDir"
        if ($baselineExit -ne 0) { throw "Workspace baseline install failed: $baselineExit" }
        if ((Get-ItemProperty -LiteralPath $registry).DisplayVersion -ne $BaselineVersion) { throw 'Baseline registration version does not match.' }
        $report.baseline_version = $BaselineVersion
        $report.baseline_installer_sha256 = (Get-FileHash -LiteralPath $BaselineInstaller).Hash.ToLowerInvariant()
        Test-InstalledRuntime 'baseline-self-test.json' $BaselineVersion
        $report.baseline_self_test = $true
        $installExit = Run-Installer $setup "/S /NS /UPDATE /D=$installDir"
        $report.version_upgrade_mode = $BaselineVersion + ' -> ' + $testVersion
    } else {
        $installExit = Run-Installer $setup "/S /NS /D=$installDir"
    }
    if ($installExit -ne 0) { throw "Workspace install failed: $installExit" }
    $info = Get-ItemProperty -LiteralPath $registry
    if ($info.DisplayVersion -ne $testVersion) { throw 'NSIS version registration mismatch.' }
    $checked = 0
    foreach ($mapping in @(@('src-tauri\bundle-docs','docs'), @('src-tauri\bundle-licenses','licenses'))) {
        $sourceRoot = (Resolve-Path -LiteralPath $mapping[0]).Path
        foreach ($source in Get-ChildItem -LiteralPath $sourceRoot -File -Recurse) {
            $relative = $source.FullName.Substring($sourceRoot.Length + 1)
            $installed = Join-Path (Join-Path $installDir $mapping[1]) $relative
            if (-not (Test-Path -LiteralPath $installed)) { throw "Missing installed resource: $($mapping[1])/$relative" }
            if ((Get-FileHash -LiteralPath $source.FullName).Hash -ne (Get-FileHash -LiteralPath $installed).Hash) { throw "Installed resource hash mismatch: $relative" }
            $checked++
        }
    }
    $report.matching_resource_files = $checked
    if (Test-Path -LiteralPath (Join-Path $installDir 'engine')) { throw 'Obsolete Python engine remains in installed application.' }
    $report.no_legacy_engine = $true
    Test-InstalledRuntime 'installed-self-test.json' $testVersion
    $report.installed_self_test = $true
    if ($BaselineInstaller) {
        if (-not (Test-Path -LiteralPath (Join-Path $dataDir 'preserve.txt'))) { throw 'Upgrade removed the user-data sentinel.' }
        $report.version_upgrade_and_data_retention = $true
    } else {
        $reinstallExit = Run-Installer $setup "/S /NS /UPDATE /D=$installDir"
        if ($reinstallExit -ne 0) { throw "Same-version NSIS update-mode reinstall failed: $reinstallExit" }
        $report.same_version_update_mode_reinstall = $true
    }
    $uninstallExit = Run-Installer (Join-Path $installDir 'uninstall.exe') '/S'
    if ($uninstallExit -ne 0) { throw "Workspace uninstall failed: $uninstallExit" }
    for ($attempt=0; $attempt -lt 40 -and (Test-Path -LiteralPath $registry); $attempt++) { Start-Sleep -Milliseconds 250 }
    if ((Test-Path -LiteralPath $registry) -or (Test-Path -LiteralPath (Join-Path $installDir "$fixtureName.exe"))) { throw 'Workspace uninstall did not clean its executable/registration.' }
    if (-not (Test-Path -LiteralPath (Join-Path $dataDir 'preserve.txt'))) { throw 'User-data sentinel was removed.' }
    $report.uninstalled_and_data_retained = $true
    $report.user_inno_untouched = (Snapshot-OldInstall) -eq $oldSnapshot
    $report.user_preview_untouched = @($previewPids | Where-Object { -not (Get-Process -Id $_ -ErrorAction SilentlyContinue) }).Count -eq 0
    if (-not $report.user_inno_untouched -or -not $report.user_preview_untouched) { throw 'User environment preservation check failed.' }
    $report.ok = $true
} finally {
    [IO.File]::WriteAllText((Join-Path $testDir 'report.json'), ($report | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    Pop-Location
}
$report | ConvertTo-Json -Depth 8
