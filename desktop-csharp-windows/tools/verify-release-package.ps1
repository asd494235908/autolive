[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$PackageRoot,

    [string]$SymbolsRoot,

    [switch]$RequireSigned,

    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [OperatingSystem]::IsWindows()) {
    throw 'This release package verifier only supports Windows.'
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

function Resolve-RegularFile([string]$Path, [string]$DisplayName) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Missing required file: $DisplayName."
    }

    $item = Get-Item -LiteralPath $Path
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Required file cannot be a reparse point: $DisplayName."
    }

    if ($item.Length -le 0) {
        throw "Required file cannot be empty: $DisplayName."
    }

    return $item
}

function Assert-RelativePath([string]$RelativePath, [string]$ExpectedPrefix) {
    if ([string]::IsNullOrWhiteSpace($RelativePath)) {
        throw 'Manifest path cannot be empty.'
    }

    $normalized = $RelativePath.Replace('/', [char]92)
    $unsafe = [IO.Path]::IsPathFullyQualified($normalized) -or ($normalized -match '(^|\\)\.\.(\\|$)') -or ($normalized -match '[\x00-\x1F]')
    if ($unsafe) {
        throw "Manifest path is unsafe: $RelativePath."
    }

    if (-not $normalized.StartsWith($ExpectedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Manifest path is outside the expected package area: $RelativePath."
    }

    return $normalized
}

function Get-ManifestJson([string]$ManifestPath, [string]$DisplayName) {
    $manifestFile = Resolve-RegularFile $ManifestPath $DisplayName
    try {
        return (Get-Content -LiteralPath $manifestFile.FullName -Raw | ConvertFrom-Json)
    }
    catch {
        throw "Invalid JSON manifest: $DisplayName."
    }
}

function Test-HashAndSize([IO.FileInfo]$File, [long]$ExpectedSize, [string]$ExpectedHash, [string]$DisplayName) {
    if ($File.Length -ne $ExpectedSize) {
        throw "Size mismatch for $DisplayName."
    }

    $actualHash = (Get-FileHash -LiteralPath $File.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $ExpectedHash.ToLowerInvariant()) {
        throw "SHA-256 mismatch for $DisplayName."
    }
}

function Get-SignatureStatus([IO.FileInfo]$File, [string]$RelativePath) {
    $signature = Get-AuthenticodeSignature -LiteralPath $File.FullName
    $status = [string]$signature.Status
    if ($RequireSigned -and $status -ne 'Valid') {
        throw "Authenticode signature is not valid for $RelativePath ($status)."
    }

    return [ordered]@{
        path = $RelativePath
        status = $status
    }
}

$package = Resolve-Directory $PackageRoot 'PackageRoot'
$null = Resolve-Directory (Join-Path $package 'runtime') 'RuntimeRoot'
$null = Resolve-Directory (Join-Path $package 'runtime/gpu') 'GpuRuntimeRoot'
$null = Resolve-Directory (Join-Path $package 'runtime/winrt') 'WinRtRuntimeRoot'
$rootNames = @(
    'GpAutoLive.Contracts.dll',
    'GpAutoLive.Core.dll',
    'GpAutoLive.deps.json',
    'GpAutoLive.dll',
    'GpAutoLive.exe',
    'GpAutoLive.Media.dll',
    'GpAutoLive.runtimeconfig.json',
    'GpAutoLive.Windows.dll'
)

$rootChildren = @(Get-ChildItem -LiteralPath $package)
$unexpectedRootDirectories = @($rootChildren | Where-Object {
        $_.PSIsContainer -and $_.Name -ne 'runtime'
    })
if ($unexpectedRootDirectories.Count -gt 0) {
    throw "Unexpected directories in package root: $($unexpectedRootDirectories.Name -join ', ')."
}

$rootFiles = @($rootChildren | Where-Object { -not $_.PSIsContainer })
$unexpectedRoot = @($rootFiles | Where-Object { $rootNames -notcontains $_.Name })
if ($unexpectedRoot.Count -gt 0) {
    throw "Unexpected files in package root: $($unexpectedRoot.Name -join ', ')."
}

$rootReport = [Collections.Generic.List[object]]::new()
foreach ($name in $rootNames) {
    $file = Resolve-RegularFile (Join-Path $package $name) $name
    $relative = $name.Replace('\', '/')
    if ($file.Extension -in @('.exe', '.dll')) {
        $rootReport.Add((Get-SignatureStatus $file $relative))
    }
    else {
        $rootReport.Add([ordered]@{ path = $relative; status = 'NotApplicable' })
    }
}

function Verify-ListedManifest([string]$ManifestRelativePath, [string]$ExpectedPrefix, [string]$BaseRelativePath = '') {
    $manifestPath = Join-Path $package ($ManifestRelativePath.Replace('/', [IO.Path]::DirectorySeparatorChar))
    $manifest = Get-ManifestJson $manifestPath $ManifestRelativePath
    if ([int]$manifest.schema_version -ne 1) {
        throw "Unsupported schema version in $ManifestRelativePath."
    }

    $entries = if ($null -ne $manifest.PSObject.Properties['files']) {
        @($manifest.files)
    }
    elseif ($null -ne $manifest.PSObject.Properties['resources']) {
        @($manifest.resources)
    }
    else {
        @()
    }
    if ($entries.Count -eq 0) {
        throw "Manifest contains no resources: $ManifestRelativePath."
    }

    $report = [Collections.Generic.List[object]]::new()
    foreach ($entry in $entries) {
        $relative = if ($null -ne $entry.PSObject.Properties['path']) {
            [string]$entry.path
        }
        else {
            [string]$entry.relative_path
        }
        $normalized = Assert-RelativePath $relative $ExpectedPrefix
        $packageRelative = if ([string]::IsNullOrWhiteSpace($BaseRelativePath)) {
            $normalized
        }
        else {
            Join-Path $BaseRelativePath $normalized
        }
        $absolute = Join-Path $package ($packageRelative.Replace('\', [IO.Path]::DirectorySeparatorChar))
        $file = Resolve-RegularFile $absolute $packageRelative
        $expectedSize = if ($null -ne $entry.PSObject.Properties['size']) {
            [long]$entry.size
        }
        else {
            [long]$entry.size_bytes
        }
        $expectedHash = [string]$entry.sha256
        if ([string]::IsNullOrWhiteSpace($expectedHash) -or $expectedHash -notmatch '^[0-9a-fA-F]{64}$') {
            throw "Manifest SHA-256 is invalid: $normalized."
        }

        Test-HashAndSize $file $expectedSize $expectedHash $packageRelative
        $signatureStatus = if ($file.Extension -in @('.exe', '.dll')) {
            (Get-SignatureStatus $file $packageRelative).status
        }
        else {
            'NotApplicable'
        }
        $report.Add([ordered]@{
                path = $packageRelative.Replace([char]92, '/')
                size_bytes = $file.Length
                sha256 = $expectedHash.ToLowerInvariant()
                signature_status = $signatureStatus
            })
    }

    return $report
}

$gpuReport = Verify-ListedManifest 'runtime/gpu/manifest.json' "runtime$([char]92)gpu$([char]92)"
$winrtReport = Verify-ListedManifest 'runtime/winrt/manifest.json' "runtime$([char]92)winrt$([char]92)"
$mediaRoot = Resolve-Directory (Join-Path $package 'runtime/media') 'MediaRuntimeRoot'
$mediaVersionDirectories = @(Get-ChildItem -LiteralPath $mediaRoot -Directory)
if ($mediaVersionDirectories.Count -ne 1) {
    throw 'The package must contain exactly one media runtime version with a manifest.'
}

$mediaVersionDirectory = $mediaVersionDirectories[0]
$invalidMediaVersionDirectory = (($mediaVersionDirectory.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) -or (-not (Test-Path -LiteralPath (Join-Path $mediaVersionDirectory.FullName 'manifest.json') -PathType Leaf))
if ($invalidMediaVersionDirectory) {
    throw 'The package must contain exactly one media runtime version with a manifest.'
}
$mediaManifestRelativePath = "runtime/media/$($mediaVersionDirectory.Name)/manifest.json"
$mediaManifestPath = Join-Path $mediaVersionDirectory.FullName 'manifest.json'
$mediaManifest = Get-ManifestJson $mediaManifestPath $mediaManifestRelativePath
if (($mediaManifest.platform -ne 'windows') -or ($mediaManifest.architecture -ne 'x64')) {
    throw 'Media runtime manifest must target Windows x64.'
}
$mediaVersion = [string]$mediaManifest.runtime_version
$invalidMediaVersion = [string]::IsNullOrWhiteSpace($mediaVersion) -or ($mediaVersion -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$')
if ($invalidMediaVersion) {
    throw 'Media runtime manifest version is invalid.'
}
if ($mediaVersion -ne $mediaVersionDirectory.Name) {
    throw 'Media runtime manifest version does not match its directory.'
}
$mediaReport = Verify-ListedManifest $mediaManifestRelativePath "bin$([char]92)" "runtime/media/$($mediaVersionDirectory.Name)"
$legalRoot = Join-Path $mediaVersionDirectory.FullName 'legal'
$missingLegal = (-not (Test-Path -LiteralPath $legalRoot -PathType Container)) -or (@(Get-ChildItem -LiteralPath $legalRoot -Recurse -File).Count -eq 0)
if ($missingLegal) {
    throw 'Media runtime license materials are missing.'
}

$symbolsReport = $null
if (-not [string]::IsNullOrWhiteSpace($SymbolsRoot)) {
    $symbols = Resolve-Directory $SymbolsRoot 'SymbolsRoot'
    $symbolNames = @(
        'GpAutoLive.Contracts.pdb',
        'GpAutoLive.Contracts.xml',
        'GpAutoLive.Core.pdb',
        'GpAutoLive.Core.xml',
        'GpAutoLive.Media.pdb',
        'GpAutoLive.pdb',
        'GpAutoLive.Windows.pdb'
    )
    $symbolFiles = @(Get-ChildItem -LiteralPath $symbols -File)
    $unexpectedSymbolDirectories = @(Get-ChildItem -LiteralPath $symbols -Directory)
    if ($unexpectedSymbolDirectories.Count -gt 0) {
        throw "Unexpected directories in symbols root: $($unexpectedSymbolDirectories.Name -join ', ')."
    }
    $unexpectedSymbols = @($symbolFiles | Where-Object { $symbolNames -notcontains $_.Name })
    if ($unexpectedSymbols.Count -gt 0) {
        throw "Unexpected files in symbols root: $($unexpectedSymbols.Name -join ', ')."
    }

    $symbolsReport = [Collections.Generic.List[object]]::new()
    foreach ($name in $symbolNames) {
        $file = Resolve-RegularFile (Join-Path $symbols $name) $name
        $symbolsReport.Add([ordered]@{ path = $name; size_bytes = $file.Length })
    }
}

$report = [ordered]@{
    schema_version = 1
    package_root_files = $rootReport
    gpu_manifest_files = $gpuReport
    winrt_manifest_files = $winrtReport
    media_manifest_files = $mediaReport
    symbols_files = $symbolsReport
    require_signed = [bool]$RequireSigned
    verified_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
}

$json = $report | ConvertTo-Json -Depth 8
if (-not [string]::IsNullOrWhiteSpace($OutputPath)) {
    $output = [IO.Path]::GetFullPath($OutputPath)
    $parent = Split-Path -Parent $output
    if ([string]::IsNullOrWhiteSpace($parent)) {
        throw 'OutputPath must include a directory.'
    }

    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }

    $temporary = "$output.$([Guid]::NewGuid().ToString('N')).tmp"
    try {
        [IO.File]::WriteAllText($temporary, $json, [Text.UTF8Encoding]::new($false))
        Move-Item -LiteralPath $temporary -Destination $output -Force
    }
    finally {
        if (Test-Path -LiteralPath $temporary) {
            Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
        }
    }
}

$json
