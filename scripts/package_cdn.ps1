$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $root
try {
    $version = (Get-Content package.json -Raw -Encoding UTF8 | ConvertFrom-Json).version
    $zip = Join-Path $root "dist/QingkanLive-$version-cdn.zip"
    if (Test-Path -LiteralPath $zip) { throw 'This version CDN archive already exists; do not overwrite published artifacts.' }
    $cdn = Join-Path $root 'dist/cdn/qingkan-live'
    Compress-Archive -LiteralPath (Join-Path $cdn $version), (Join-Path $cdn 'latest.json'), (Join-Path $cdn 'UPLOAD-INSTRUCTIONS.md') -DestinationPath $zip -CompressionLevel Fastest
    $archive = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        $names = @($archive.Entries | Where-Object { -not $_.FullName.EndsWith('/') } | ForEach-Object { $_.FullName.Replace('\','/') })
        $expected = @('latest.json','UPLOAD-INSTRUCTIONS.md',"$version/QingkanLive-$version-windows-x64-setup.exe","$version/QingkanLive-$version-windows-x64-setup.exe.sig","$version/QingkanLive-$version-windows-x64-setup.exe.sha256")
        if ($names.Count -ne 5 -or @(Compare-Object $expected $names).Count) { throw 'CDN archive contains unexpected files' }
        foreach ($entry in $archive.Entries | Where-Object { -not $_.FullName.EndsWith('/') }) {
            $stream = $entry.Open()
            try { $digest = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)).ToLowerInvariant() }
            finally { $stream.Dispose() }
            if ($digest -ne (Get-FileHash -LiteralPath (Join-Path $cdn $entry.FullName) -Algorithm SHA256).Hash.ToLowerInvariant()) { throw 'CDN archive entry hash mismatch' }
        }
    } finally { $archive.Dispose() }
    $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($zip))" | Set-Content ($zip + '.sha256') -Encoding ascii
    [ordered]@{ ok=$true; version=$version; entries=$names; sha256=$hash; bytes=(Get-Item $zip).Length } | ConvertTo-Json -Depth 4 | Set-Content "build/cdn-archive-$version.json" -Encoding utf8
    Write-Output $zip
} finally { Pop-Location }
