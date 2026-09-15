[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,

    [switch]$RequireSigned,

    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [OperatingSystem]::IsWindows()) {
    throw 'The C# Windows NSIS verifier only supports Windows.'
}

$resolved = [IO.Path]::GetFullPath($InstallerPath)
if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
    throw 'InstallerPath must point to an existing file.'
}
$item = Get-Item -LiteralPath $resolved
if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw 'InstallerPath cannot be a reparse point.'
}
if ($item.Extension -ne '.exe' -or $item.Length -le 0 -or $item.Length -gt 700MB) {
    throw 'InstallerPath is not a bounded Windows executable.'
}
$versionInfo = $item.VersionInfo
if ($versionInfo.ProductName -ne 'GpAutoLive' -or $versionInfo.FileDescription -ne 'GpAutoLive Windows 安装程序') {
    throw 'Installer version metadata is invalid.'
}
$signature = Get-AuthenticodeSignature -LiteralPath $resolved
if ($RequireSigned -and $signature.Status -ne 'Valid') {
    throw "Installer Authenticode signature is not valid ($($signature.Status))."
}

$report = [ordered]@{
    schema_version = 1
    installer_path = $item.FullName
    size_bytes = $item.Length
    sha256 = (Get-FileHash -LiteralPath $resolved -Algorithm SHA256).Hash.ToLowerInvariant()
    product_name = $versionInfo.ProductName
    product_version = $versionInfo.ProductVersion
    file_version = $versionInfo.FileVersion
    signature_status = $signature.Status.ToString()
    require_signed = [bool]$RequireSigned
    verified_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
}

if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
    $target = [IO.Path]::GetFullPath($OutputPath)
    $parent = Split-Path -Parent $target
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        throw 'OutputPath parent directory must exist.'
    }
    $temporary = "$target.partial-$([Guid]::NewGuid().ToString('N'))"
    try {
        [IO.File]::WriteAllText($temporary, ($report | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
        Move-Item -LiteralPath $temporary -Destination $target -Force
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force
        }
    }
}

$report | ConvertTo-Json -Depth 4
