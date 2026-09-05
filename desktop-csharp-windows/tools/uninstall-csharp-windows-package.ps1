[CmdletBinding(SupportsShouldProcess, ConfirmImpact = 'Medium')]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallRoot,

    [Parameter(Mandatory = $true)]
    [string]$Version
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [OperatingSystem]::IsWindows()) {
    throw 'This uninstall script only supports Windows.'
}

. (Join-Path $PSScriptRoot 'csharp-windows-install-transaction-lock.ps1')

if ([string]::IsNullOrWhiteSpace($Version) -or $Version.Length -gt 64 -or $Version -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$') {
    throw 'Version has an invalid format.'
}

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

function Assert-NoReparseEntries([string]$Root) {
    $entries = @(Get-ChildItem -LiteralPath $Root -Force -Recurse -ErrorAction Stop)
    foreach ($entry in $entries) {
        if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Installed version contains a reparse point; uninstall stopped: $($entry.FullName)."
        }
    }
}

$install = Resolve-Directory $InstallRoot 'InstallRoot'
Assert-NoRunningApplication $install

$stateTool = Join-Path $PSScriptRoot 'get-csharp-windows-install-state.ps1'
if (-not (Test-Path -LiteralPath $stateTool -PathType Leaf)) {
    throw 'The installation state verifier is missing beside the uninstall script.'
}
try {
    $stateRaw = (& $stateTool -InstallRoot $install 2>&1 | Out-String)
    $state = $stateRaw | ConvertFrom-Json
}
catch {
    throw 'Installation state could not be verified; uninstall stopped without deleting a version.'
}
if ([string]$state.status -ne 'healthy') {
    throw 'Installation state is not healthy; uninstall stopped without deleting a version.'
}
$targetState = @($state.versions | Where-Object { [string]$_.version -eq $Version })
if ($targetState.Count -ne 1 -or [string]$targetState[0].status -ne 'verified') {
    throw 'The uninstall target is not a uniquely verified installed version.'
}
if ([bool]$targetState[0].active) {
    throw 'The active version cannot be uninstalled; roll back or activate another version first.'
}
if ([bool]$targetState[0].rollback_candidate) {
    throw 'The current rollback candidate cannot be uninstalled; activate another version first.'
}

$versions = Resolve-Directory (Join-Path $install 'versions') 'VersionsRoot'
$target = Join-Path $versions $Version
if (-not (Test-Path -LiteralPath $target -PathType Container)) {
    throw "Installed version was not found: $Version."
}
$targetItem = Get-Item -LiteralPath $target
if (($targetItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw 'Installed version cannot be a reparse point.'
}
Assert-NoReparseEntries $target

$currentPath = Join-Path $install 'current.json'
try {
    $currentFile = Get-Item -LiteralPath $currentPath -Force -ErrorAction Stop
}
catch [Management.Automation.ItemNotFoundException] {
    $currentFile = $null
}
if ($null -ne $currentFile) {
    if ($currentFile.PSIsContainer -or (($currentFile.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
        throw 'current.json cannot be a reparse point or directory.'
    }
    $raw = Get-Content -LiteralPath $currentFile.FullName -Raw
    if ([string]::IsNullOrWhiteSpace($raw) -or $raw.Length -gt 64KB) {
        throw 'current.json is invalid or exceeds the bounded pointer size.'
    }
    try {
        $current = $raw | ConvertFrom-Json
    }
    catch {
        throw 'current.json is invalid; uninstall stopped without changing the installation.'
    }
    if ([string]::Equals([string]$current.active_version, $Version, [StringComparison]::Ordinal)) {
        throw 'The active version cannot be uninstalled; roll back or activate another version first.'
    }
}

if ($WhatIfPreference) {
    [ordered]@{
        schema_version = 1
        uninstall_version = $Version
        install_root = $install
        target = $target
        status = 'what_if'
    } | ConvertTo-Json -Depth 4
    return
}

if (-not $PSCmdlet.ShouldProcess($target, "Remove installed version $Version")) {
    throw 'Uninstall was cancelled before removing the version.'
}

Remove-Item -LiteralPath $target -Recurse -Force
[ordered]@{
    schema_version = 1
    uninstall_version = $Version
    install_root = $install
    target = $target
    status = 'uninstalled'
} | ConvertTo-Json -Depth 4
