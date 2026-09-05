[CmdletBinding(SupportsShouldProcess, ConfirmImpact = 'Medium')]
param(
    [Parameter(Mandatory = $true)]
    [string]$PackageRoot,

    [Parameter(Mandatory = $true)]
    [string]$InstallRoot,

    [Parameter(Mandatory = $true)]
    [string]$Version,

    [string]$SymbolsRoot,

    [switch]$RequireSigned
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [OperatingSystem]::IsWindows()) {
    throw 'This installer skeleton only supports Windows.'
}

. (Join-Path $PSScriptRoot 'csharp-windows-install-transaction-lock.ps1')

$invalidVersion = [string]::IsNullOrWhiteSpace($Version) -or ($Version.Length -gt 64) -or ($Version -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$')
if ($invalidVersion) {
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

function Resolve-OrCreateDirectory([string]$Path, [string]$Name) {
    $resolved = [IO.Path]::GetFullPath($Path)
    if (Test-Path -LiteralPath $resolved) {
        return Resolve-Directory $resolved $Name
    }

    $parent = Split-Path -Parent $resolved
    if ([string]::IsNullOrWhiteSpace($parent)) {
        throw "$Name must have a parent directory."
    }

    if ($WhatIfPreference) {
        if (Test-Path -LiteralPath $parent -PathType Container) {
            $null = Resolve-Directory $parent "$Name parent"
        }

        return $resolved
    }

    $null = Resolve-Directory $parent "$Name parent"
    New-Item -ItemType Directory -Path $resolved -Force | Out-Null
    return Resolve-Directory $resolved $Name
}

function Assert-NoRunningApplication([string]$Root) {
    $prefix = $Root.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    $processes = @(Get-Process -Name 'GpAutoLive' -ErrorAction SilentlyContinue)
    foreach ($process in $processes) {
        try {
            $path = $process.MainModule.FileName
            if ([string]::IsNullOrWhiteSpace($path)) {
                throw 'Application path could not be inspected.'
            }

            $normalized = [IO.Path]::GetFullPath($path)
            $isTargetProcess = $normalized.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -or [string]::Equals($normalized, (Join-Path $Root 'GpAutoLive.exe'), [StringComparison]::OrdinalIgnoreCase)
            if ($isTargetProcess) {
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

function Assert-Version([string]$Candidate, [string]$Name) {
    if ([string]::IsNullOrWhiteSpace($Candidate) -or $Candidate.Length -gt 64 -or $Candidate -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$') {
        throw "$Name has an invalid format."
    }
}

function Copy-RegularTree([string]$Source, [string]$Destination) {
    $null = New-Item -ItemType Directory -Force -Path $Destination
    foreach ($entry in Get-ChildItem -LiteralPath $Source -Force) {
        if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Package entry cannot be a reparse point: $($entry.Name)."
        }

        $target = Join-Path $Destination $entry.Name
        if ($entry.PSIsContainer) {
            Copy-RegularTree $entry.FullName $target
        }
        else {
            Copy-Item -LiteralPath $entry.FullName -Destination $target
        }
    }
}

function Get-OptionalRegularFile([string]$Path, [string]$Name) {
    try {
        $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    }
    catch [Management.Automation.ItemNotFoundException] {
        return $null
    }

    if ($item.PSIsContainer -or (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0)) {
        throw "$Name cannot be a reparse point or directory."
    }

    return $item
}

function Read-CurrentPointer([string]$Path) {
    $file = Get-OptionalRegularFile $Path 'current.json'
    if ($null -eq $file) {
        return $null
    }

    if ($file.Length -le 0 -or $file.Length -gt 64KB) {
        throw 'Existing current.json is invalid or exceeds the bounded pointer size; installation stopped without changing it.'
    }

    try {
        $pointer = Get-Content -LiteralPath $file.FullName -Raw | ConvertFrom-Json
    }
    catch {
        throw 'Existing current.json is invalid; installation stopped without changing it.'
    }

    $schema = 0
    try {
        $schema = [int]$pointer.schema_version
    }
    catch {
        throw 'Existing current.json has an invalid schema; installation stopped without changing it.'
    }
    if ($schema -ne 1) {
        throw 'Existing current.json has an unsupported schema; installation stopped without changing it.'
    }

    $active = [string]$pointer.active_version
    Assert-Version $active 'Existing active_version'
    if ([string]$pointer.relative_path -ne "versions/$active") {
        throw 'Existing current.json relative_path does not match active_version; installation stopped without changing it.'
    }

    $previousProperty = $pointer.PSObject.Properties['previous_version']
    if ($null -eq $previousProperty) {
        throw 'Existing current.json is missing previous_version; installation stopped without changing it.'
    }
    $previous = $null
    if ($null -ne $previousProperty.Value -and -not [string]::IsNullOrWhiteSpace([string]$previousProperty.Value)) {
        $previous = [string]$previousProperty.Value
        Assert-Version $previous 'Existing previous_version'
        if ([string]::Equals($previous, $active, [StringComparison]::Ordinal)) {
            throw 'Existing current.json previous_version cannot equal active_version; installation stopped without changing it.'
        }
    }

    [pscustomobject]@{
        active_version = $active
        previous_version = $previous
    }
}

$package = Resolve-Directory $PackageRoot 'PackageRoot'
$install = Resolve-OrCreateDirectory $InstallRoot 'InstallRoot'
Assert-NoRunningApplication $install

$verifier = Join-Path $PSScriptRoot 'verify-release-package.ps1'
if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
    throw 'The release package verifier is missing beside the installer.'
}

$verifyParameters = @{
    PackageRoot = $package
}
if (-not [string]::IsNullOrWhiteSpace($SymbolsRoot)) {
    $verifyParameters.SymbolsRoot = $SymbolsRoot
}
if ($RequireSigned) {
    $verifyParameters.RequireSigned = $true
}

& $verifier @verifyParameters | Out-Null
if (-not $?) {
    throw 'Release package verification failed; installation was not started.'
}

$versions = Resolve-OrCreateDirectory (Join-Path $install 'versions') 'VersionsRoot'
$target = Join-Path $versions $Version
if (Test-Path -LiteralPath $target) {
    throw "Target version already exists; installation will not overwrite it: $Version."
}

$currentPath = Join-Path $install 'current.json'
$current = Read-CurrentPointer $currentPath
$previousVersion = if ($null -eq $current) { $null } else { [string]$current.active_version }
if ($null -ne $current) {
    $activeDirectory = Join-Path $versions $current.active_version
    if (-not (Test-Path -LiteralPath $activeDirectory -PathType Container)) {
        throw 'Existing active version is missing; installation stopped without changing it.'
    }
    $activeItem = Get-Item -LiteralPath $activeDirectory -Force
    if (($activeItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'Existing active version cannot be a reparse point; installation stopped without changing it.'
    }
}

if ($WhatIfPreference) {
    [ordered]@{
        schema_version = 1
        installed_version = $Version
        previous_version = $previousVersion
        install_root = $install
        active_pointer = $currentPath
        status = 'what_if'
    } | ConvertTo-Json -Depth 4
    return
}

$staging = Join-Path $versions ('.staging-' + [Guid]::NewGuid().ToString('N'))
$moved = $false
$activated = $false
$pointerTemporary = Join-Path $install ('.current-' + [Guid]::NewGuid().ToString('N') + '.tmp')
try {
    if (-not $PSCmdlet.ShouldProcess($staging, "Stage version $Version")) {
        throw 'Installation was cancelled before staging.'
    }
    Copy-RegularTree $package $staging

    if (-not $PSCmdlet.ShouldProcess($target, "Activate version $Version")) {
        throw 'Installation was cancelled before version activation.'
    }
    Move-Item -LiteralPath $staging -Destination $target
    $moved = $true

    $pointer = [ordered]@{
        schema_version = 1
        active_version = $Version
        relative_path = "versions/$Version"
        previous_version = $previousVersion
        updated_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
    }
    [IO.File]::WriteAllText(
        $pointerTemporary,
        ($pointer | ConvertTo-Json -Depth 4),
        [Text.UTF8Encoding]::new($false))

    if (-not $PSCmdlet.ShouldProcess($currentPath, "Atomically activate version $Version")) {
        throw 'Installation was cancelled before pointer activation.'
    }

    if (Test-Path -LiteralPath $currentPath -PathType Leaf) {
        [IO.File]::Move($pointerTemporary, $currentPath, $true)
    }
    else {
        [IO.File]::Move($pointerTemporary, $currentPath)
    }
    $activated = $true

    [ordered]@{
        schema_version = 1
        installed_version = $Version
        previous_version = $previousVersion
        install_root = $install
        active_pointer = $currentPath
        status = 'activated'
    } | ConvertTo-Json -Depth 4
}
finally {
    if (Test-Path -LiteralPath $pointerTemporary) {
        Remove-Item -LiteralPath $pointerTemporary -Force -ErrorAction SilentlyContinue
    }

    if (-not $moved -and (Test-Path -LiteralPath $staging)) {
        Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
    }

    if (-not $activated -and $moved -and (Test-Path -LiteralPath $target -PathType Container)) {
        Remove-Item -LiteralPath $target -Recurse -Force -ErrorAction SilentlyContinue
    }
}
