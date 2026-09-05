[CmdletBinding(SupportsShouldProcess, ConfirmImpact = 'Medium')]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallRoot,

    [string]$Version,

    [switch]$RequireSigned
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [OperatingSystem]::IsWindows()) {
    throw 'This rollback script only supports Windows.'
}

. (Join-Path $PSScriptRoot 'csharp-windows-install-transaction-lock.ps1')

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

function Assert-Version([string]$Candidate) {
    if ([string]::IsNullOrWhiteSpace($Candidate) -or $Candidate.Length -gt 64 -or $Candidate -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$') {
        throw 'Version has an invalid format.'
    }
}

function Assert-NoRunningApplication([string]$Root) {
    $prefix = $Root.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    foreach ($process in @(Get-Process -Name 'GpAutoLive' -ErrorAction SilentlyContinue)) {
        try {
            $path = $process.MainModule.FileName
            if ([string]::IsNullOrWhiteSpace($path)) {
                throw 'Application path could not be inspected.'
            }

            $normalized = [IO.Path]::GetFullPath($path)
            $targetExe = Join-Path $Root 'GpAutoLive.exe'
            if ($normalized.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -or [string]::Equals($normalized, $targetExe, [StringComparison]::OrdinalIgnoreCase)) {
                throw 'GpAutoLive is running from the target installation root.'
            }
        }
        catch [ComponentModel.Win32Exception] {
            throw 'A running GpAutoLive process could not be inspected safely.'
        }
        catch [InvalidOperationException] {
            throw 'A running GpAutoLive process could not be inspected safely.'
        }
    }
}

function Read-CurrentPointer([string]$Path) {
    try {
        $file = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    }
    catch [Management.Automation.ItemNotFoundException] {
        throw 'current.json is missing; rollback stopped without changing the installation.'
    }

    if ($file.PSIsContainer -or (($file.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
        throw 'current.json cannot be a reparse point or directory.'
    }

    $raw = Get-Content -LiteralPath $file.FullName -Raw
    if ([string]::IsNullOrWhiteSpace($raw) -or $raw.Length -gt 64KB) {
        throw 'current.json is invalid or exceeds the bounded pointer size.'
    }

    try {
        $pointer = $raw | ConvertFrom-Json
    }
    catch {
        throw 'current.json is invalid; rollback stopped without changing the installation.'
    }

    $active = [string]$pointer.active_version
    if ([string]::IsNullOrWhiteSpace($active)) {
        throw 'current.json has no active_version; rollback stopped without changing the installation.'
    }
    Assert-Version $active
    if ([string]$pointer.relative_path -ne "versions/$active") {
        throw 'current.json relative_path does not match active_version.'
    }

    return $pointer
}

$install = Resolve-Directory $InstallRoot 'InstallRoot'
Assert-NoRunningApplication $install
$versions = Resolve-Directory (Join-Path $install 'versions') 'VersionsRoot'
$currentPath = Join-Path $install 'current.json'
$current = Read-CurrentPointer $currentPath
$activeVersion = [string]$current.active_version
$targetVersion = if ([string]::IsNullOrWhiteSpace($Version)) { [string]$current.previous_version } else { $Version }
Assert-Version $targetVersion

if ([string]::Equals($targetVersion, $activeVersion, [StringComparison]::Ordinal)) {
    throw 'Rollback target is already the active version.'
}

$target = Join-Path $versions $targetVersion
if (-not (Test-Path -LiteralPath $target -PathType Container)) {
    throw "Rollback target version is not installed: $targetVersion."
}
$targetItem = Get-Item -LiteralPath $target
if (($targetItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw 'Rollback target cannot be a reparse point.'
}

$verifier = Join-Path $PSScriptRoot 'verify-release-package.ps1'
if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
    throw 'The release package verifier is missing beside the rollback script.'
}
$verifyParameters = @{ PackageRoot = $target }
if ($RequireSigned) {
    $verifyParameters.RequireSigned = $true
}
& $verifier @verifyParameters | Out-Null
if (-not $?) {
    throw 'Rollback target package verification failed; current.json was not changed.'
}

if ($WhatIfPreference) {
    [ordered]@{
        schema_version = 1
        active_version = $activeVersion
        rollback_version = $targetVersion
        install_root = $install
        active_pointer = $currentPath
        status = 'what_if'
    } | ConvertTo-Json -Depth 4
    return
}

$pointerTemporary = Join-Path $install ('.current-' + [Guid]::NewGuid().ToString('N') + '.tmp')
try {
    $pointer = [ordered]@{
        schema_version = 1
        active_version = $targetVersion
        relative_path = "versions/$targetVersion"
        previous_version = $activeVersion
        updated_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
        rollback_from = $activeVersion
    }
    [IO.File]::WriteAllText($pointerTemporary, ($pointer | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))

    if (-not $PSCmdlet.ShouldProcess($currentPath, "Atomically rollback to version $targetVersion")) {
        throw 'Rollback was cancelled before pointer activation.'
    }

    [IO.File]::Move($pointerTemporary, $currentPath, $true)
    [ordered]@{
        schema_version = 1
        active_version = $targetVersion
        previous_version = $activeVersion
        install_root = $install
        active_pointer = $currentPath
        status = 'rolled_back'
    } | ConvertTo-Json -Depth 4
}
finally {
    if (Test-Path -LiteralPath $pointerTemporary) {
        Remove-Item -LiteralPath $pointerTemporary -Force -ErrorAction SilentlyContinue
    }
}
