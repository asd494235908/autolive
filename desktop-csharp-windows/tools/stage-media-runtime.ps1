[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory = $true)]
    [string]$MpvRoot,

    [Parameter(Mandatory = $true)]
    [string]$InstallRoot,

    [string]$FfmpegRoot,

    [string]$PortAudioRoot,

    [string]$RuntimeVersion = '1.0.0'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Resolve-ExistingDirectory([string]$Path, [string]$ParameterName) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "$ParameterName cannot be empty."
    }

    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
        throw "$ParameterName must be an existing directory."
    }

    $item = Get-Item -LiteralPath $resolved
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "$ParameterName cannot be a reparse point."
    }

    return $resolved.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
}

function Resolve-RegularFile([string]$Path, [string]$Name) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Missing media runtime resource: $Name."
    }

    $item = Get-Item -LiteralPath $Path
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Media runtime resource cannot be a reparse point: $Name."
    }

    if ($item.Length -le 0 -or $item.Length -gt 512MB) {
        throw "Media runtime resource size is outside the allowed range: $Name."
    }

    return $item
}

function Get-Resource([string]$Name, [string]$Root) {
    return Resolve-RegularFile (Join-Path $Root $Name) $Name
}

if ([string]::IsNullOrWhiteSpace($RuntimeVersion) -or $RuntimeVersion.Length -gt 64 -or $RuntimeVersion -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$') {
    throw 'RuntimeVersion has an invalid format.'
}

$mpvRootResolved = Resolve-ExistingDirectory $MpvRoot 'MpvRoot'
$ffmpegRootResolved = if ([string]::IsNullOrWhiteSpace($FfmpegRoot)) {
    $mpvRootResolved
} else {
    Resolve-ExistingDirectory $FfmpegRoot 'FfmpegRoot'
}
$portAudioRootResolved = if ([string]::IsNullOrWhiteSpace($PortAudioRoot)) {
    $null
} else {
    Resolve-ExistingDirectory $PortAudioRoot 'PortAudioRoot'
}
$installRootResolved = Resolve-ExistingDirectory $InstallRoot 'InstallRoot'

$sourceItems = [ordered]@{}
$sourceItems['mpv.exe'] = Get-Resource 'mpv.exe' $mpvRootResolved
$sourceItems['ffmpeg.exe'] = Get-Resource 'ffmpeg.exe' $ffmpegRootResolved
$sourceItems['ffprobe.exe'] = Get-Resource 'ffprobe.exe' $ffmpegRootResolved
$shaderSource = Join-Path $PSScriptRoot '..\src\GpAutoLive.Media\Resources\gpu83.hook'
$sourceItems['gpu83.hook'] = Resolve-RegularFile $shaderSource 'gpu83.hook'
$portAudioDll = if ($null -eq $portAudioRootResolved) { $null } else { Join-Path $portAudioRootResolved 'portaudio_x64.dll' }
if ($null -ne $portAudioDll) {
    $sourceItems['portaudio_x64.dll'] = Resolve-RegularFile $portAudioDll 'portaudio_x64.dll'
}
$d3dcompiler = Join-Path $mpvRootResolved 'd3dcompiler_43.dll'
if (Test-Path -LiteralPath $d3dcompiler -PathType Leaf) {
    $sourceItems['d3dcompiler_43.dll'] = Resolve-RegularFile $d3dcompiler 'd3dcompiler_43.dll'
}

$mediaRoot = Join-Path $installRootResolved 'runtime\media'
$targetVersion = Join-Path $mediaRoot $RuntimeVersion
if (Test-Path -LiteralPath $targetVersion) {
    throw "Target runtime version already exists; choose a new RuntimeVersion to avoid overwriting: $RuntimeVersion."
}

$stagingRoot = Join-Path $mediaRoot ('.staging-' + [Guid]::NewGuid().ToString('N'))
$stagingVersion = Join-Path $stagingRoot $RuntimeVersion
$stagingBin = Join-Path $stagingVersion 'bin'
$stagingLegal = Join-Path $stagingVersion 'legal'

try {
    $null = New-Item -ItemType Directory -Force -Path $stagingBin

    $resources = [System.Collections.Generic.List[object]]::new()
    foreach ($entry in $sourceItems.GetEnumerator()) {
        $name = [string]$entry.Key
        $source = [IO.FileInfo]$entry.Value
        $destination = Join-Path $stagingBin $name
        if ($PSCmdlet.ShouldProcess($destination, "Copy $name")) {
            Copy-Item -LiteralPath $source.FullName -Destination $destination
        }

        $resources.Add([ordered]@{
            name = $name
            relative_path = "bin/$name"
            size_bytes = $source.Length
            sha256 = (Get-FileHash -LiteralPath $source.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        })
    }

    $legalSource = Join-Path $mpvRootResolved 'legal'
    if (Test-Path -LiteralPath $legalSource -PathType Container) {
        if ($PSCmdlet.ShouldProcess($stagingLegal, 'Copy media license materials')) {
            Copy-Item -LiteralPath $legalSource -Destination $stagingLegal -Recurse
        }
    }

    if ($null -ne $portAudioRootResolved) {
        $portAudioLegal = Join-Path $stagingLegal 'portaudio'
        $legalFiles = @('LICENSE.txt', 'README.md')
        foreach ($legalFile in $legalFiles) {
            $sourceLegalFile = Join-Path $portAudioRootResolved $legalFile
            if (Test-Path -LiteralPath $sourceLegalFile -PathType Leaf) {
                if ($PSCmdlet.ShouldProcess($portAudioLegal, "Copy PortAudio $legalFile")) {
                    $null = New-Item -ItemType Directory -Force -Path $portAudioLegal
                    Copy-Item -LiteralPath $sourceLegalFile -Destination (Join-Path $portAudioLegal $legalFile)
                }
            }
        }
    }

    $manifest = [ordered]@{
        schema_version = 1
        runtime_version = $RuntimeVersion
        platform = 'windows'
        architecture = 'x64'
        resources = $resources
    }
    $manifestPath = Join-Path $stagingVersion 'manifest.json'
    $manifestJson = $manifest | ConvertTo-Json -Depth 5
    if ($PSCmdlet.ShouldProcess($manifestPath, 'Write resource manifest')) {
        [IO.File]::WriteAllText($manifestPath, $manifestJson, [Text.UTF8Encoding]::new($false))
    }

    if ($PSCmdlet.ShouldProcess($targetVersion, 'Atomically commit media runtime version')) {
        $null = New-Item -ItemType Directory -Force -Path $mediaRoot
        Move-Item -LiteralPath $stagingVersion -Destination $targetVersion
    }

    $totalBytes = 0L
    foreach ($resource in $resources) {
        $totalBytes += [long]$resource['size_bytes']
    }

    [ordered]@{
        runtime_version = $RuntimeVersion
        target = $targetVersion
        resources = @($resources | ForEach-Object { $_.name })
        total_bytes = $totalBytes
    } | ConvertTo-Json -Depth 4
}
finally {
    if (Test-Path -LiteralPath $stagingRoot) {
        Remove-Item -LiteralPath $stagingRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
