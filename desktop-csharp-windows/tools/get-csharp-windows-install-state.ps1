[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'None')]
param(
    [Parameter(Mandatory = $true)]
    [string]$InstallRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$maxPointerBytes = 64KB
$maxInstallEntries = 5000
$maxVersions = 100
$maxVerifierOutputBytes = 128KB
$maxReportBytes = 128KB

function New-Failure([string]$Code) {
    throw [InvalidOperationException]::new($Code)
}

function Resolve-ExistingDirectory([string]$Path, [string]$Name) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        New-Failure "$Name`_missing"
    }

    try {
        $resolved = [IO.Path]::GetFullPath($Path)
    }
    catch {
        New-Failure "$Name`_invalid"
    }

    if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
        New-Failure "$Name`_missing"
    }

    try {
        $item = Get-Item -LiteralPath $resolved -ErrorAction Stop
    }
    catch {
        New-Failure "$Name`_unreadable"
    }

    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        New-Failure "$Name`_reparse_point"
    }

    return $resolved.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
}

function Resolve-RegularFile([string]$Path, [string]$Code) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        New-Failure $Code
    }

    try {
        $item = Get-Item -LiteralPath $Path -ErrorAction Stop
    }
    catch {
        New-Failure $Code
    }

    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        New-Failure "${Code}_reparse_point"
    }

    if ($item.Length -le 0 -or $item.Length -gt $maxPointerBytes) {
        New-Failure "${Code}_size"
    }

    return $item
}

function Assert-Version([string]$Version, [string]$Code) {
    if ([string]::IsNullOrWhiteSpace($Version) -or $Version.Length -gt 64 -or $Version -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$') {
        New-Failure $Code
    }
}

function Get-PropertyValue([object]$Object, [string]$Name) {
    if ($null -eq $Object) {
        return $null
    }

    if ($Object -is [Collections.IDictionary] -and $Object.Contains($Name)) {
        return $Object[$Name]
    }

    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        return $null
    }

    return $property.Value
}

function Read-CurrentPointer([string]$Path) {
    $file = Resolve-RegularFile $Path 'current_pointer'
    try {
        $raw = Get-Content -LiteralPath $file.FullName -Raw -ErrorAction Stop
        $pointer = $raw | ConvertFrom-Json
    }
    catch {
        New-Failure 'current_pointer_invalid'
    }

    $schema = Get-PropertyValue $pointer 'schema_version'
    $schemaNumber = 0
    try {
        $schemaNumber = [int]$schema
    }
    catch {
        New-Failure 'current_pointer_schema'
    }
    if ($null -eq $schema -or $schemaNumber -ne 1) {
        New-Failure 'current_pointer_schema'
    }

    $active = [string](Get-PropertyValue $pointer 'active_version')
    $relativePath = [string](Get-PropertyValue $pointer 'relative_path')
    if ([string]::IsNullOrWhiteSpace($active)) {
        New-Failure 'current_pointer_active_missing'
    }
    Assert-Version $active 'current_pointer_active_invalid'
    if ($relativePath -ne "versions/$active") {
        New-Failure 'current_pointer_relative_path'
    }

    $previousProperty = $pointer.PSObject.Properties['previous_version']
    if ($null -eq $previousProperty) {
        New-Failure 'current_pointer_previous_missing'
    }

    $previous = $null
    if ($null -ne $previousProperty.Value -and -not [string]::IsNullOrWhiteSpace([string]$previousProperty.Value)) {
        $previous = [string]$previousProperty.Value
        Assert-Version $previous 'current_pointer_previous_invalid'
    }

    [pscustomobject]@{
        active_version = $active
        previous_version = $previous
    }
}

function Assert-NoReparseEntries([string]$Root) {
    $entries = @(Get-ChildItem -LiteralPath $Root -Force -Recurse -ErrorAction Stop)
    if ($entries.Count -gt $maxInstallEntries) {
        New-Failure 'install_entry_limit'
    }

    foreach ($entry in $entries) {
        if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            New-Failure 'install_reparse_point'
        }
    }
}

function Get-VersionDirectories([string]$VersionsRoot) {
    $children = @(Get-ChildItem -LiteralPath $VersionsRoot -Force -ErrorAction Stop)
    if ($children.Count -gt $maxVersions) {
        New-Failure 'version_limit'
    }

    $directories = [Collections.Generic.List[object]]::new()
    foreach ($entry in $children) {
        if (-not $entry.PSIsContainer) {
            New-Failure 'versions_unexpected_file'
        }
        if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            New-Failure 'version_reparse_point'
        }
        Assert-Version $entry.Name 'version_name_invalid'
        $directories.Add($entry)
    }

    return $directories
}

function Assert-InstallationRootShape([string]$Root) {
    $allowed = @('current.json', 'versions')
    foreach ($entry in @(Get-ChildItem -LiteralPath $Root -Force -ErrorAction Stop)) {
        if ($allowed -notcontains $entry.Name) {
            New-Failure 'install_unexpected_entry'
        }
    }
}

function Invoke-PackageVerification([string]$PackageRoot, [string]$VerifierPath) {
    try {
        $raw = (& $VerifierPath -PackageRoot $PackageRoot 2>&1 | Out-String)
        if ($raw.Length -le 0 -or $raw.Length -gt $maxVerifierOutputBytes) {
            return [ordered]@{
                status = 'invalid'
                code = 'package_report_limit'
            }
        }

        $verification = $raw | ConvertFrom-Json
        $groups = @('package_root_files', 'gpu_manifest_files', 'winrt_manifest_files', 'media_manifest_files')
        $counts = [ordered]@{}
        $signatureCounts = [ordered]@{
            valid = 0
            unsigned = 0
            invalid = 0
            other = 0
        }

        foreach ($group in $groups) {
            $items = @(Get-PropertyValue $verification $group)
            if ($items.Count -eq 0) {
                return [ordered]@{
                    status = 'invalid'
                    code = 'package_manifest_missing'
                }
            }
            $counts[$group] = $items.Count
            foreach ($item in $items) {
                $signatureStatus = [string](Get-PropertyValue $item 'status')
                if ([string]::IsNullOrWhiteSpace($signatureStatus)) {
                    $signatureStatus = [string](Get-PropertyValue $item 'signature_status')
                }
                switch ($signatureStatus) {
                    'Valid' { $signatureCounts.valid++ }
                    'NotSigned' { $signatureCounts.unsigned++ }
                    'UnknownError' { $signatureCounts.invalid++ }
                    'HashMismatch' { $signatureCounts.invalid++ }
                    'NotApplicable' { }
                    default {
                        if (-not [string]::IsNullOrWhiteSpace($signatureStatus)) {
                            $signatureCounts.other++
                        }
                    }
                }
            }
        }

        [ordered]@{
            status = 'verified'
            code = 'package_verified'
            manifests = $counts
            signatures = $signatureCounts
        }
    }
    catch {
        [ordered]@{
            status = 'invalid'
            code = 'package_verification_failed'
        }
    }
}

function New-BaseReport([string]$Status, [string]$Code) {
    [ordered]@{
        schema_version = 1
        tool = 'get-csharp-windows-install-state'
        mode = if ($WhatIfPreference) { 'what_if' } else { 'read_only' }
        status = $Status
        code = $Code
        active = $null
        rollback = $null
        versions = @()
        checked_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
    }
}

if (-not [OperatingSystem]::IsWindows()) {
    $json = (New-BaseReport 'invalid' 'not_windows') | ConvertTo-Json -Depth 8 -Compress
    $json
    return
}

$report = $null
try {
    $install = Resolve-ExistingDirectory $InstallRoot 'install_root'
    Assert-NoReparseEntries $install
    Assert-InstallationRootShape $install

    $versionsRoot = Resolve-ExistingDirectory (Join-Path $install 'versions') 'versions_root'
    $currentPath = Join-Path $install 'current.json'
    $pointer = Read-CurrentPointer $currentPath
    $versionDirectories = @(Get-VersionDirectories $versionsRoot)
    $verifier = Join-Path $PSScriptRoot 'verify-release-package.ps1'
    if (-not (Test-Path -LiteralPath $verifier -PathType Leaf)) {
        New-Failure 'verifier_missing'
    }

    $activeDirectory = $versionDirectories | Where-Object { $_.Name -eq $pointer.active_version }
    if ($null -eq $activeDirectory) {
        New-Failure 'active_version_missing'
    }

    $previousDirectory = $null
    if ($null -ne $pointer.previous_version) {
        if ($pointer.previous_version -eq $pointer.active_version) {
            New-Failure 'current_pointer_previous_active'
        }
        $previousDirectory = $versionDirectories | Where-Object { $_.Name -eq $pointer.previous_version }
        if ($null -eq $previousDirectory) {
            New-Failure 'rollback_version_missing'
        }
    }

    $versionReports = [Collections.Generic.List[object]]::new()
    foreach ($directory in $versionDirectories) {
        $package = Invoke-PackageVerification $directory.FullName $verifier
        $isActive = $directory.Name -eq $pointer.active_version
        $isRollback = $null -ne $pointer.previous_version -and $directory.Name -eq $pointer.previous_version
        $manifestSummary = Get-PropertyValue $package 'manifests'
        $signatureSummary = Get-PropertyValue $package 'signatures'
        $versionReports.Add([ordered]@{
                version = $directory.Name
                active = $isActive
                rollback_candidate = $isRollback
                status = $package.status
                code = $package.code
                manifests = $manifestSummary
                signatures = $signatureSummary
            })
    }

    $activeReport = $versionReports | Where-Object { $_.active }
    if ($null -eq $activeReport -or $activeReport.status -ne 'verified') {
        New-Failure 'active_package_invalid'
    }

    $rollbackReport = $null
    if ($null -ne $pointer.previous_version) {
        $rollbackReport = $versionReports | Where-Object { $_.rollback_candidate }
        if ($null -eq $rollbackReport -or $rollbackReport.status -ne 'verified') {
            New-Failure 'rollback_package_invalid'
        }
    }

    $invalidVersions = @($versionReports | Where-Object { $_.status -ne 'verified' }).Count
    $overallStatus = if ($invalidVersions -eq 0) { 'healthy' } else { 'invalid' }
    $overallCode = if ($invalidVersions -eq 0) { 'installation_verified' } else { 'installed_version_invalid' }
    $report = New-BaseReport $overallStatus $overallCode
    $report.active = [ordered]@{
        version = $pointer.active_version
        status = 'verified'
    }
    $report.rollback = if ($null -eq $pointer.previous_version) {
        [ordered]@{
            available = $false
            version = $null
            status = 'none'
        }
    }
    else {
        [ordered]@{
            available = $true
            version = $pointer.previous_version
            status = 'verified'
        }
    }
    $report.versions = $versionReports
}
catch {
    $code = [string]$_.Exception.Message
    if ([string]::IsNullOrWhiteSpace($code) -or $code.Length -gt 80 -or $code -notmatch '^[a-z0-9_]+$') {
        $code = 'state_check_failed'
    }
    $report = New-BaseReport 'invalid' $code
}

$json = $report | ConvertTo-Json -Depth 10 -Compress
if ($json.Length -gt $maxReportBytes) {
    $bounded = New-BaseReport 'invalid' 'report_limit'
    $json = $bounded | ConvertTo-Json -Depth 8 -Compress
}
$json
