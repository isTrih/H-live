# Adapted from the installed official Sites upload-source.ps1.
# Preserve UTC when PowerShell 7.5+ ConvertFrom-Json returns DateTime instead of string.
#Requires -Version 7.0
param(
    [Parameter(Mandatory)][Alias('Artifact')][string]$Source,
    [Parameter(Mandatory)][string]$Ticket
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
try {
    $ticketData = Get-Content -LiteralPath $Ticket -Raw | ConvertFrom-Json
    $upload = $ticketData.upload
    $uri = [uri]$upload.url
    if ($upload.method -cne 'PUT' -or $uri.Scheme -ne 'https' -or $uri.UserInfo -or
        ([DateTimeOffset]$upload.expires_at) -le [DateTimeOffset]::UtcNow) {
        throw 'Invalid or expired upload ticket'
    }
    $file = Get-Item -LiteralPath $Source
    if ($file.PSIsContainer -or $file.Length -ne [long]$upload.headers.'content-length' -or
        $file.Length -ne [long]$ticketData.size -or
        (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -cne $ticketData.sha256) {
        throw 'Source ZIP does not match ticket'
    }
    $headers = @{}
    foreach ($property in $upload.headers.PSObject.Properties) {
        $headers[$property.Name] = [string]$property.Value
    }
    $response = Invoke-WebRequest -Uri $uri -Method Put -Headers $headers `
        -InFile $file.FullName -MaximumRedirection 0 -TimeoutSec 120 -SkipHttpErrorCheck
    $status = [int]$response.StatusCode
    Write-Output "Upload HTTP $status"
    if ($status -eq 409) {
        Write-Output 'Object may already exist; MCP must verify its size and SHA-256.'
    } elseif ($status -lt 200 -or $status -ge 300) {
        exit 1
    }
} catch {
    # PowerShell HTTP exceptions can contain signed URLs; do not print them.
    Write-Output 'Upload failed; check file, ticket expiry and connectivity, then retry the same valid ticket.'
    exit 1
}
