$ErrorActionPreference = 'Stop'
$releaseRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$version = (Get-Content -LiteralPath (Join-Path $releaseRoot 'package.json') -Raw | ConvertFrom-Json).version
if ($version -ne '0.3.6') { throw 'Legacy CDN is permanently pinned to 0.3.6. Later releases publish only to the internal Sites endpoint.' }
$source = Join-Path $releaseRoot 'dist/cdn/qingkan-live'
$output = [IO.Path]::GetFullPath((Join-Path $releaseRoot 'dist/legacy-cdn/qingkan-live'))
if (-not $output.StartsWith($releaseRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Legacy CDN output escaped workspace.' }
if (Test-Path -LiteralPath $output) { Remove-Item -LiteralPath $output -Recurse -Force }
New-Item -ItemType Directory -Path (Join-Path $output $version) -Force | Out-Null
$filename = "QingkanLive-${version}-windows-x64-setup.exe"
foreach ($suffix in @('', '.sig', '.sha256')) { Copy-Item -LiteralPath (Join-Path $source "$version/$filename$suffix") -Destination (Join-Path $output $version) }
$manifest = Get-Content -LiteralPath (Join-Path $source 'latest.json') -Raw | ConvertFrom-Json
$manifest.platforms.'windows-x86_64'.url = "https://assets.checkpoint321.com/lilith/qingkan-live/$version/$filename"
[IO.File]::WriteAllText((Join-Path $output 'latest.json'), ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
$instructions = "旧客户端迁移包 $version`n`n将 $version 目录先上传到 https://assets.checkpoint321.com/lilith/qingkan-live/，确认安装包可下载后再上传 latest.json。`n安装包与内网站点完全相同，签名不变。升级后的客户端自动改用 https://qingkan-live.internal.lilithsites.com/latest.json 检查更新，需要公司网络或 VPN。`n此 ZIP 不含私钥。请勿上传工程源码、配置、日志或 .release-secrets。`n"
[IO.File]::WriteAllText((Join-Path $output 'UPLOAD-INSTRUCTIONS.txt'), $instructions, [Text.UTF8Encoding]::new($false))
$zip = Join-Path $releaseRoot "dist/QingkanLive-${version}-legacy-cdn.zip"
Compress-Archive -Path (Join-Path $output '*') -DestinationPath $zip -Force
Write-Output $zip
