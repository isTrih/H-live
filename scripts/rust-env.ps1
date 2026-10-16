# Dot-source before Cargo. Prefer the installed Windows SDK; the project-local
# NuGet SDK is an optional fallback. This only changes the current process.
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Visual Studio C++ build tools were not found.' }
$visualStudio = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $visualStudio) { throw 'Install the Visual Studio Desktop development with C++ workload and a Windows 10/11 SDK.' }
$msvc = Get-ChildItem -LiteralPath (Join-Path $visualStudio 'VC\Tools\MSVC') -Directory | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
if (-not $msvc) { throw 'MSVC compiler tools were not found.' }
$kits = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -ErrorAction SilentlyContinue
$systemSdk = if ($kits.KitsRoot10) { $kits.KitsRoot10 } else { Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10' }
$sdkHeaders = $null
$sdkLibraries = $null
$sdkVersion = $null
$candidates = @(
    @{ Headers=$systemSdk; Libraries=$null },
    @{ Headers=(Join-Path $projectRoot 'packaging\tools\windows-sdk-cpp\c'); Libraries=(Join-Path $projectRoot 'packaging\tools\windows-sdk-x64\c') }
)
foreach ($candidate in $candidates) {
    $includeRoot = Join-Path $candidate.Headers 'Include'
    if (-not (Test-Path -LiteralPath $includeRoot)) { continue }
    foreach ($versionDirectory in (Get-ChildItem -LiteralPath $includeRoot -Directory | Where-Object Name -Match '^\d+\.\d+\.\d+\.\d+$' | Sort-Object { [version]$_.Name } -Descending)) {
        $version = $versionDirectory.Name
        $libraries = if ($candidate.Libraries) { $candidate.Libraries } else { Join-Path $candidate.Headers ('Lib\' + $version) }
        $required = @((Join-Path $versionDirectory.FullName 'um\Windows.h'), (Join-Path $libraries 'um\x64\kernel32.lib'),
                      (Join-Path $libraries 'ucrt\x64\ucrt.lib'), (Join-Path $candidate.Headers ('bin\' + $version + '\x64\rc.exe')))
        if (@($required | Where-Object { -not (Test-Path -LiteralPath $_) }).Count -eq 0) {
            $sdkHeaders=$candidate.Headers; $sdkLibraries=$libraries; $sdkVersion=$version
            break
        }
    }
    if ($sdkVersion) { break }
}
if (-not $sdkVersion) { throw 'Install a Windows 10/11 SDK with x64 headers, libraries and resource compiler; see docs/release/BUILD.md.' }
$compilerBin = Join-Path $msvc.FullName 'bin\Hostx64\x64'
$sdkBin = Join-Path $sdkHeaders ('bin\' + $sdkVersion + '\x64')
$env:PATH = $compilerBin + ';' + $sdkBin + ';' + $env:PATH
$env:LIB = (Join-Path $msvc.FullName 'lib\x64') + ';' + (Join-Path $sdkLibraries 'um\x64') + ';' + (Join-Path $sdkLibraries 'ucrt\x64')
$includePaths = @((Join-Path $msvc.FullName 'include')) + @(@('ucrt','shared','um','winrt','cppwinrt') | ForEach-Object { Join-Path $sdkHeaders ('Include\' + $sdkVersion + '\' + $_) })
$env:INCLUDE = $includePaths -join ';'
$env:WindowsSdkDir = $sdkHeaders + '\'
$env:WindowsSDKVersion = $sdkVersion + '\'
$env:WindowsSDKLibVersion = $sdkVersion + '\'
$env:VCToolsInstallDir = $msvc.FullName + '\'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $compilerBin 'link.exe'
$env:RC = Join-Path $sdkBin 'rc.exe'
$env:CARGO_HOME = Join-Path $projectRoot 'packaging\tools\cargo-home'
