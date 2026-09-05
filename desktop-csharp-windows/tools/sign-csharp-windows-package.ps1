[CmdletBinding(SupportsShouldProcess, ConfirmImpact = 'Medium')]
param(
    [Parameter(Mandatory = $true)]
    [string]$PackageRoot,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$CertificateThumbprint,

    [string]$TimestampUrl = 'https://timestamp.digicert.com',

    [string]$SignToolPath,

    [string]$SymbolsRoot,

    [switch]$RequireSigned
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [OperatingSystem]::IsWindows()) {
    throw 'This signing script only supports Windows.'
}

$approvedTimestampHosts = @(
    'timestamp.digicert.com',
    'rfc3161timestamp.globalsign.com',
    'timestamp.sectigo.com'
)
$maxToolOutputBytes = 256KB

function Resolve-Directory([string]$Path, [string]$Name) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "$Name cannot be empty."
    }
    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
        throw "$Name must point to an existing directory."
    }
    $item = Get-Item -LiteralPath $resolved
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "$Name cannot be a reparse point."
    }
    return $resolved.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
}

function Resolve-TimestampUri([string]$Value) {
    $uri = [Uri]::new($Value)
    if ($uri.Scheme -ne 'https' -or ($uri.Port -ne -1 -and $uri.Port -ne 443) -or $approvedTimestampHosts -notcontains $uri.Host.ToLowerInvariant()) {
        throw 'TimestampUrl must use HTTPS on an approved timestamp host.'
    }
    return $uri
}

function Find-SignTool([string]$Candidate) {
    if (-not [string]::IsNullOrWhiteSpace($Candidate)) {
        $resolved = [IO.Path]::GetFullPath($Candidate)
        if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
            throw 'SignToolPath must point to signtool.exe.'
        }
        return $resolved
    }

    $roots = @()
    if (-not [string]::IsNullOrWhiteSpace(${env:ProgramFiles(x86)})) {
        $roots += Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    }
    if (-not [string]::IsNullOrWhiteSpace($env:ProgramFiles)) {
        $roots += Join-Path $env:ProgramFiles 'Windows Kits\10\bin'
    }
    foreach ($root in $roots | Select-Object -Unique) {
        if (Test-Path -LiteralPath $root -PathType Container) {
            $found = Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue |
                Sort-Object Name -Descending |
                ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
                Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
                Select-Object -First 1
            if ($null -ne $found) {
                return [IO.Path]::GetFullPath($found)
            }
        }
    }
    throw 'signtool.exe was not found; install the Windows SDK or pass -SignToolPath.'
}

function Get-Certificate([string]$Thumbprint) {
    $normalized = $Thumbprint.Replace(' ', '').ToUpperInvariant()
    $stores = @('Cert:\CurrentUser\My', 'Cert:\LocalMachine\My')
    foreach ($store in $stores) {
        $certificate = Get-ChildItem -LiteralPath $store -ErrorAction SilentlyContinue |
            Where-Object { $_.Thumbprint -eq $normalized } |
            Select-Object -First 1
        if ($null -ne $certificate) {
            if (-not $certificate.HasPrivateKey) {
                throw "Signing certificate $normalized has no private key."
            }
            return $certificate
        }
    }
    throw "Signing certificate $normalized was not found in the current-user or local-machine store."
}

function Get-PeFiles([string]$Root) {
    $files = @(Get-ChildItem -LiteralPath $Root -File -Recurse -Force -ErrorAction Stop |
        Where-Object { $_.Extension -in @('.exe', '.dll') })
    foreach ($file in $files) {
        if (($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "PE file cannot be a reparse point: $($file.FullName)."
        }
    }
    return $files | Sort-Object FullName
}

function Invoke-SignTool([string]$Tool, [string]$Thumbprint, [Uri]$Timestamp, [string]$FilePath) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Tool
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in @('/sha1', $Thumbprint, '/fd', 'SHA256', '/tr', $Timestamp.AbsoluteUri, '/td', 'SHA256', $FilePath)) {
        $null = $start.ArgumentList.Add($argument)
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        if (-not $process.Start()) {
            throw 'signtool process did not start.'
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(120000)) {
            try { $process.Kill($true) } catch { }
            throw "signtool timed out for $FilePath."
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($stdout.Length -gt $maxToolOutputBytes -or $stderr.Length -gt $maxToolOutputBytes) {
            throw 'signtool output exceeded the bounded limit.'
        }
        if ($process.ExitCode -ne 0) {
            throw "signtool failed for $FilePath with exit code $($process.ExitCode): $stderr"
        }
    }
    finally {
        $process.Dispose()
    }
}

function Update-ManifestHashes([string]$ManifestPath, [string]$BaseDirectory) {
    $raw = Get-Content -LiteralPath $ManifestPath -Raw
    if ($raw.Length -gt 256KB) {
        throw "Manifest is larger than the bounded limit: $ManifestPath."
    }
    $manifest = $raw | ConvertFrom-Json
    $entriesProperty = $manifest.PSObject.Properties['files']
    if ($null -eq $entriesProperty) {
        $entriesProperty = $manifest.PSObject.Properties['resources']
    }
    if ($null -eq $entriesProperty) {
        throw "Manifest has no hash entries: $ManifestPath."
    }

    foreach ($entry in @($entriesProperty.Value)) {
        $relativeProperty = $entry.PSObject.Properties['path']
        if ($null -eq $relativeProperty) {
            $relativeProperty = $entry.PSObject.Properties['relative_path']
        }
        $hashProperty = $entry.PSObject.Properties['sha256']
        if ($null -eq $relativeProperty -or $null -eq $hashProperty) {
            throw "Manifest hash entry is incomplete: $ManifestPath."
        }
        $relative = ([string]$relativeProperty.Value).Replace('/', [IO.Path]::DirectorySeparatorChar).Replace('\', [IO.Path]::DirectorySeparatorChar)
        $filePath = [IO.Path]::GetFullPath((Join-Path $BaseDirectory $relative))
        if (-not (Test-Path -LiteralPath $filePath -PathType Leaf)) {
            throw "Manifest resource is missing: $relative."
        }
        $hashProperty.Value = (Get-FileHash -LiteralPath $filePath -Algorithm SHA256).Hash.ToLowerInvariant()
        $sizeProperty = $entry.PSObject.Properties['size_bytes']
        if ($null -eq $sizeProperty) {
            $sizeProperty = $entry.PSObject.Properties['size']
        }
        if ($null -ne $sizeProperty) {
            $sizeProperty.Value = (Get-Item -LiteralPath $filePath).Length
        }
    }

    $temporary = "$ManifestPath.$([Guid]::NewGuid().ToString('N')).tmp"
    try {
        [IO.File]::WriteAllText($temporary, ($manifest | ConvertTo-Json -Depth 20), [Text.UTF8Encoding]::new($false))
        [IO.File]::Move($temporary, $ManifestPath, $true)
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
        }
    }
}

$package = Resolve-Directory $PackageRoot 'PackageRoot'
$timestamp = Resolve-TimestampUri $TimestampUrl
$thumbprint = $CertificateThumbprint.Replace(' ', '').ToUpperInvariant()
$verifier = Join-Path $PSScriptRoot 'verify-release-package.ps1'
if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
    throw 'The release package verifier is missing beside the signing script.'
}

$verifyParameters = @{ PackageRoot = $package }
if (-not [string]::IsNullOrWhiteSpace($SymbolsRoot)) {
    $verifyParameters.SymbolsRoot = $SymbolsRoot
}
& $verifier @verifyParameters | Out-Null
if (-not $?) {
    throw 'Release package verification failed; signing was not started.'
}

$peFiles = @(Get-PeFiles $package)
if ($peFiles.Count -eq 0) {
    throw 'No PE files were found in the package.'
}

$unsignedFiles = @(
    foreach ($file in $peFiles) {
        $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
        if ($signature.Status -ne 'Valid') {
            $file
        }
    }
)

if ($WhatIfPreference) {
    [ordered]@{
        schema_version = 1
        package_root = $package
        timestamp_url = $timestamp.AbsoluteUri
        pe_file_count = $peFiles.Count
        files_requiring_signature = $unsignedFiles.Count
        status = 'what_if'
    } | ConvertTo-Json -Depth 5
    return
}

$null = Get-Certificate $thumbprint
$tool = Find-SignTool $SignToolPath
foreach ($file in $unsignedFiles) {
    if (-not $PSCmdlet.ShouldProcess($file.FullName, 'Authenticode sign PE file')) {
        throw 'Signing was cancelled before all files were signed.'
    }
    Invoke-SignTool $tool $thumbprint $timestamp $file.FullName
}

$gpuManifest = Join-Path $package 'runtime\gpu\manifest.json'
$winrtManifest = Join-Path $package 'runtime\winrt\manifest.json'
if (Test-Path -LiteralPath $gpuManifest -PathType Leaf) {
    Update-ManifestHashes $gpuManifest $package
}
if (Test-Path -LiteralPath $winrtManifest -PathType Leaf) {
    Update-ManifestHashes $winrtManifest $package
}
$mediaRoot = Join-Path $package 'runtime\media'
if (Test-Path -LiteralPath $mediaRoot -PathType Container) {
    foreach ($mediaManifest in @(Get-ChildItem -LiteralPath $mediaRoot -Filter 'manifest.json' -File -Recurse -Force)) {
        Update-ManifestHashes $mediaManifest.FullName $mediaManifest.Directory.FullName
    }
}

$signedVerifyParameters = @{ PackageRoot = $package; RequireSigned = $true }
if (-not [string]::IsNullOrWhiteSpace($SymbolsRoot)) {
    $signedVerifyParameters.SymbolsRoot = $SymbolsRoot
}
& $verifier @signedVerifyParameters | Out-Null
if (-not $?) {
    throw 'Signed package verification failed; package remains in the caller-provided staging directory.'
}

[ordered]@{
    schema_version = 1
    package_root = $package
    signed_files = $unsignedFiles.Count
    timestamp_url = $timestamp.AbsoluteUri
    status = 'signed_and_verified'
} | ConvertTo-Json -Depth 5
