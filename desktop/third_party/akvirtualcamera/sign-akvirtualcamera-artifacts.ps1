[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9A-Fa-f]{40}$')]
    [string] $CertificateThumbprint,

    [Parameter(Mandatory = $false)]
    [ValidatePattern('^https://')]
    [string] $TimestampUrl,

    [Parameter(Mandatory = $false)]
    [string] $RepositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
)

$ErrorActionPreference = 'Stop'

function Fail([string] $Message) {
    throw "AkVirtualCamera 签名门禁失败：$Message"
}

function Resolve-SignTool {
    $command = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }
    $sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $candidate = Get-ChildItem -LiteralPath $sdkRoot -Filter signtool.exe -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '\\(x64|x86)\\signtool\.exe$' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if ($candidate) {
        return $candidate.FullName
    }
    Fail '找不到 Windows SDK signtool.exe'
}

function Resolve-RepoFile([string] $RelativePath) {
    if ([string]::IsNullOrWhiteSpace($RelativePath) -or $RelativePath.Contains('\') -or
        $RelativePath.StartsWith('/') -or $RelativePath.Contains(':')) {
        Fail "锁文件路径不是正斜杠相对路径：$RelativePath"
    }
    $full = [IO.Path]::GetFullPath((Join-Path $RepositoryRoot ($RelativePath -replace '/', '\')))
    $root = [IO.Path]::GetFullPath($RepositoryRoot).TrimEnd('\') + '\'
    if (-not $full.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
        Fail "锁文件路径越出仓库根目录：$RelativePath"
    }
    return $full
}

$lockPath = Join-Path $RepositoryRoot 'desktop\third_party\akvirtualcamera\upstream.lock.json'
if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
    Fail "缺少锁文件：$lockPath"
}
$lock = Get-Content -Raw -LiteralPath $lockPath | ConvertFrom-Json
$artifacts = @($lock.release_requirements.artifacts)
if ($artifacts.Count -ne 7) {
    Fail "锁文件必须声明 7 个固定产物，实际为 $($artifacts.Count)"
}

$thumbprint = $CertificateThumbprint.ToUpperInvariant()
$certificate = Get-ChildItem -LiteralPath "Cert:\CurrentUser\My\$thumbprint" -ErrorAction SilentlyContinue
if (-not $certificate) {
    Fail "CurrentUser\\My 中不存在证书 $thumbprint"
}
if (-not $certificate.HasPrivateKey) {
    Fail '代码签名证书没有私钥'
}
if ($certificate.NotAfter -le (Get-Date)) {
    Fail '代码签名证书已过期'
}
$codeSigningEku = @($certificate.EnhancedKeyUsageList | Where-Object {
    $_.ObjectId -eq '1.3.6.1.5.5.7.3.3'
})
if ($codeSigningEku.Count -eq 0) {
    Fail '证书缺少代码签名 EKU（1.3.6.1.5.5.7.3.3）'
}

$signTool = Resolve-SignTool
$signed = @()
foreach ($artifact in $artifacts) {
    $artifactPath = Resolve-RepoFile $artifact.path
    if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) {
        Fail "缺少待签名产物：$($artifact.path)"
    }
    $arguments = @('sign', '/sha1', $thumbprint, '/fd', 'SHA256')
    if ($TimestampUrl) {
        $arguments += @('/tr', $TimestampUrl, '/td', 'SHA256')
    }
    $arguments += $artifactPath
    & $signTool @arguments
    if ($LASTEXITCODE -ne 0) {
        Fail "signtool sign 失败：$($artifact.path)"
    }
    & $signTool verify '/pa' '/all' $artifactPath
    if ($LASTEXITCODE -ne 0) {
        Fail "Authenticode 验证失败：$($artifact.path)"
    }

    $auth = Get-AuthenticodeSignature -LiteralPath $artifactPath
    if ($auth.Status -ne 'Valid') {
        Fail "PowerShell Authenticode 状态不是 Valid：$($artifact.path)（$($auth.Status)）"
    }
    $fileHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $artifactPath).Hash.ToLowerInvariant()
    $signaturePath = Resolve-RepoFile $artifact.signature_path
    $signatureDirectory = Split-Path -Parent $signaturePath
    New-Item -ItemType Directory -Force -Path $signatureDirectory | Out-Null
    $evidence = [ordered]@{
        schemaVersion = 1
        status = 'valid'
        artifactPath = $artifact.path
        artifactSha256 = $fileHash
        signatureStatus = [string] $auth.Status
        subject = [string] $auth.SignerCertificate.Subject
        thumbprint = [string] $auth.SignerCertificate.Thumbprint
        certificateNotAfter = $auth.SignerCertificate.NotAfter.ToUniversalTime().ToString('o')
        timestampUrl = if ($TimestampUrl) { $TimestampUrl } else { $null }
        signtool = $signTool
    }
    $temporaryEvidence = "$signaturePath.$PID.tmp"
    [IO.File]::WriteAllText($temporaryEvidence, ($evidence | ConvertTo-Json -Depth 5) + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporaryEvidence -Destination $signaturePath -Force
    $signed += [pscustomobject]@{ path = $artifact.path; sha256 = $fileHash; signaturePath = $artifact.signature_path }
}

$signed | ConvertTo-Json -Depth 5
Write-Output '签名完成；请把上述产物/证据 SHA-256 写回 upstream.lock.json，再运行 --require-release-ready。'
