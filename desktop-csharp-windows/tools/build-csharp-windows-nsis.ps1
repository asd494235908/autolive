[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$PackageRoot,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^v[0-9]{1,6}$')]
    [string]$Version,

    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\artifacts'),

    [string]$RuntimeInstallerPath = (Join-Path $PSScriptRoot '..\artifacts\cache\windowsdesktop-runtime-10.0.11-win-x64.exe'),

    [string]$MakeNsisPath,

    [switch]$DownloadRuntime,

    [switch]$DevelopmentUnsigned,

    [ValidateSet('Production', 'CloudTest')]
    [string]$ControlPlaneProfile = 'Production'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [OperatingSystem]::IsWindows()) {
    throw 'The C# Windows NSIS builder only supports Windows.'
}

$runtimeUrl = 'https://builds.dotnet.microsoft.com/dotnet/WindowsDesktop/10.0.11/windowsdesktop-runtime-10.0.11-win-x64.exe'
$runtimeSha512 = '4dbf26b0b78f55c5f59a46c3c81327b23a04f449f7ac6798204dcd19d99459258936daaede61d1b8c1ba523d6c26bf68bac86b3371d22e67cef235edbdc2f26c'
$cloudTestBaseUri = 'http://101.96.208.132:9090'
$controlPlaneProfileMarker = if ($ControlPlaneProfile -eq 'CloudTest') { 'cloud-test-v1' } else { 'production-v1' }
if ($ControlPlaneProfile -eq 'CloudTest' -and -not $DevelopmentUnsigned) {
    throw 'CloudTest control-plane packages must use -DevelopmentUnsigned and cannot be formal release artifacts.'
}
$package = [IO.Path]::GetFullPath($PackageRoot)
$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
$runtime = [IO.Path]::GetFullPath($RuntimeInstallerPath)
$source = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\installer\windows\GpAutoLive.nsi'))
$icon = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\src\GpAutoLive.App\Assets\app-icon.ico'))
$verifyPackage = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'verify-release-package.ps1'))

foreach ($directory in @($package)) {
    if (-not (Test-Path -LiteralPath $directory -PathType Container)) {
        throw "Required directory does not exist: $directory"
    }
    if (((Get-Item -LiteralPath $directory).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Required directory cannot be a reparse point: $directory"
    }
}
foreach ($file in @($source, $icon, $verifyPackage)) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        throw "Required file does not exist: $file"
    }
}

if ($DevelopmentUnsigned) {
    & $verifyPackage -PackageRoot $package | Out-Null
}
else {
    & $verifyPackage -PackageRoot $package -RequireSigned -RequireReleaseReadyLegal | Out-Null
}

if (-not (Test-Path -LiteralPath $runtime -PathType Leaf)) {
    if (-not $DownloadRuntime) {
        throw 'The pinned .NET 10 Desktop Runtime installer is missing. Pass -DownloadRuntime to fetch the official Microsoft artifact.'
    }
    $runtimeDirectory = Split-Path -Parent $runtime
    New-Item -ItemType Directory -Path $runtimeDirectory -Force | Out-Null
    $partial = "$runtime.partial-$([Guid]::NewGuid().ToString('N'))"
    try {
        Invoke-WebRequest -Uri $runtimeUrl -OutFile $partial -MaximumRedirection 0 -UseBasicParsing
        $downloaded = Get-Item -LiteralPath $partial
        if ($downloaded.Length -le 0 -or $downloaded.Length -gt 150MB) {
            throw 'Downloaded .NET Desktop Runtime has an invalid size.'
        }
        Move-Item -LiteralPath $partial -Destination $runtime
    }
    finally {
        if (Test-Path -LiteralPath $partial) {
            Remove-Item -LiteralPath $partial -Force
        }
    }
}

$runtimeItem = Get-Item -LiteralPath $runtime
if (($runtimeItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or $runtimeItem.Length -le 0 -or $runtimeItem.Length -gt 150MB) {
    throw 'The .NET Desktop Runtime installer is not a valid regular file.'
}
$actualRuntimeHash = (Get-FileHash -LiteralPath $runtime -Algorithm SHA512).Hash.ToLowerInvariant()
if ($actualRuntimeHash -ne $runtimeSha512) {
    throw 'The .NET Desktop Runtime SHA-512 does not match Microsoft release metadata.'
}
$runtimeSignature = Get-AuthenticodeSignature -LiteralPath $runtime
if ($runtimeSignature.Status -ne 'Valid' -or $null -eq $runtimeSignature.SignerCertificate -or $runtimeSignature.SignerCertificate.Subject -notmatch 'Microsoft Corporation') {
    throw 'The .NET Desktop Runtime Authenticode signature is not a valid Microsoft signature.'
}

if ([string]::IsNullOrWhiteSpace($MakeNsisPath)) {
    $candidates = @(
        'C:\Program Files (x86)\NSIS\makensis.exe',
        (Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe')
    )
    $MakeNsisPath = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
if ([string]::IsNullOrWhiteSpace($MakeNsisPath) -or -not (Test-Path -LiteralPath $MakeNsisPath -PathType Leaf)) {
    throw 'NSIS makensis.exe was not found. Install NSIS 3.12 or pass -MakeNsisPath.'
}
$makeNsis = [IO.Path]::GetFullPath($MakeNsisPath)
$nsisVersion = (& $makeNsis /VERSION 2>&1 | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $nsisVersion -notmatch '^v3\.(1[2-9]|[2-9][0-9])') {
    throw "NSIS 3.12 or newer is required; found '$nsisVersion'."
}

New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
$versionNumber = [int]$Version.Substring(1)
$productVersion = "0.1.$versionNumber.0"
$profileSuffix = if ($ControlPlaneProfile -eq 'CloudTest') { '-TEST' } else { '' }
$suffix = if ($DevelopmentUnsigned) { '-UNSIGNED-LEGAL-REVIEW' } else { '-REQUIRES-OUTER-SIGNATURE' }
$installer = Join-Path $outputRoot "GpAutoLive-Setup-$Version$profileSuffix$suffix.exe"
$reportPath = "$installer.json"
if (Test-Path -LiteralPath $installer) {
    throw "Refusing to overwrite an existing installer: $installer"
}

$arguments = @(
    '/V3',
    "/DAPP_PACKAGE_ROOT=$package",
    "/DDOTNET_RUNTIME_PATH=$runtime",
    "/DOUTPUT_PATH=$installer",
    "/DPACKAGE_VERSION=$Version",
    "/DPRODUCT_VERSION=$productVersion",
    "/DAPP_ICON=$icon",
    "/DCONTROL_PLANE_PROFILE=$controlPlaneProfileMarker",
    $source
)
& $makeNsis @arguments
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $installer -PathType Leaf)) {
    throw 'NSIS compilation failed.'
}

$installerItem = Get-Item -LiteralPath $installer
$installerSignature = Get-AuthenticodeSignature -LiteralPath $installer
$report = [ordered]@{
    schema_version = 1
    status = if ($DevelopmentUnsigned) { 'development_unsigned_legal_review' } else { 'compiled_requires_outer_signature' }
    version = $Version
    installer_path = $installerItem.FullName
    installer_size_bytes = $installerItem.Length
    installer_sha256 = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
    installer_signature_status = $installerSignature.Status.ToString()
    package_root = $package
    control_plane = [ordered]@{
        profile = $ControlPlaneProfile
        marker = $controlPlaneProfileMarker
        base_uri = if ($ControlPlaneProfile -eq 'CloudTest') { $cloudTestBaseUri } else { $null }
    }
    dotnet_runtime = [ordered]@{
        version = '10.0.11'
        path = $runtime
        size_bytes = $runtimeItem.Length
        sha512 = $actualRuntimeHash
        signature_status = $runtimeSignature.Status.ToString()
        signer_subject = $runtimeSignature.SignerCertificate.Subject
    }
    nsis_version = $nsisVersion
    built_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
}
$json = $report | ConvertTo-Json -Depth 6
$temporaryReport = "$reportPath.partial-$([Guid]::NewGuid().ToString('N'))"
try {
    [IO.File]::WriteAllText($temporaryReport, $json, [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporaryReport -Destination $reportPath
}
finally {
    if (Test-Path -LiteralPath $temporaryReport) {
        Remove-Item -LiteralPath $temporaryReport -Force
    }
}

$report | ConvertTo-Json -Depth 6
