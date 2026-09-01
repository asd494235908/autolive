[CmdletBinding()]
param(
    [ValidateSet('x86', 'x64')]
    [string]$Architecture = 'x64',
    [string]$SourceArchive,
    [string]$BuildRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Get-Sha256Hex {
    param([Parameter(Mandatory = $true)][string]$Path)

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        return ([BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
    } finally {
        $stream.Dispose()
        $sha256.Dispose()
    }
}

function Apply-LoopbackPatch {
    param(
        [Parameter(Mandatory = $true)][string]$SourceDirectory,
        [Parameter(Mandatory = $true)][string]$PatchFile,
        [Parameter(Mandatory = $true)][string]$RepositoryRoot,
        [Parameter(Mandatory = $true)][string]$GitPath
    )

    Push-Location $RepositoryRoot
    try {
        $relativeSource = (Resolve-Path -LiteralPath $SourceDirectory -Relative) -replace '^[.][\\/]', ''
        $relativeSource = $relativeSource.Replace('\', '/')
        & $GitPath apply --check --whitespace=nowarn --directory=$relativeSource $PatchFile
        $checkExit = $LASTEXITCODE
        if ($checkExit -eq 0) {
            & $GitPath apply --whitespace=nowarn --directory=$relativeSource $PatchFile
            if ($LASTEXITCODE -ne 0) { throw "AkVirtualCamera source security patch failed: $LASTEXITCODE" }
            return
        }
        & $GitPath apply --reverse --check --whitespace=nowarn --directory=$relativeSource $PatchFile
        if ($LASTEXITCODE -ne 0) {
            throw "AkVirtualCamera source security patch cannot be applied or verified as already applied: $PatchFile"
        }
    } finally {
        Pop-Location
    }
}

$componentRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $SourceArchive) {
    $SourceArchive = Join-Path $componentRoot 'source/akvirtualcamera-9cf77ae6379e5f635255f4b377478d388a46a3b2.tar.gz'
}
if (-not $BuildRoot) {
    $BuildRoot = Join-Path $componentRoot 'local-build-directshow'
}

$cmake = Get-Command cmake -ErrorAction Stop
$tar = Get-Command tar -ErrorAction Stop
$git = Get-Command git -ErrorAction Stop
if (-not (Test-Path -LiteralPath $SourceArchive -PathType Leaf)) {
    throw "AkVirtualCamera source archive not found: $SourceArchive"
}
$generator = $env:CMAKE_GENERATOR
if ([string]::IsNullOrWhiteSpace($generator)) {
    throw 'CMAKE_GENERATOR must select the installed MSVC generator (for example Visual Studio 17 2022)'
}

$sourceStage = Join-Path $BuildRoot "source-$Architecture"
$cmakeBuild = Join-Path $BuildRoot "cmake-$Architecture"
New-Item -ItemType Directory -Path $sourceStage,$cmakeBuild -Force | Out-Null
& $tar.Source -xf $SourceArchive -C $sourceStage
$sourceDir = Get-ChildItem -LiteralPath $sourceStage -Directory | Select-Object -First 1
if (-not $sourceDir) {
    throw 'AkVirtualCamera source archive did not contain a top-level directory'
}

# Apply the same loopback-only hardening used by the sidecar build. The upstream
# MessageServer otherwise binds INADDR_ANY and exposes its unauthenticated IPC
# protocol to the LAN. Re-running with the same BuildRoot verifies an already
# applied patch instead of mutating the staged source twice.
$patchFile = Join-Path $componentRoot 'patches/0002-loopback-service-socket.patch'
$repositoryRoot = (Resolve-Path (Join-Path $componentRoot '..\..\..')).Path
Apply-LoopbackPatch -SourceDirectory $sourceDir.FullName -PatchFile $patchFile `
    -RepositoryRoot $repositoryRoot -GitPath $git.Source

$platform = if ($Architecture -eq 'x86') { 'Win32' } else { 'x64' }
& $cmake.Source -S $sourceDir.FullName -B $cmakeBuild -G $generator -A $platform
if ($LASTEXITCODE -ne 0) { throw "CMake configure failed: $LASTEXITCODE" }
& $cmake.Source --build $cmakeBuild --config Release --target VirtualCamera_dshow --parallel
if ($LASTEXITCODE -ne 0) { throw "CMake build failed: $LASTEXITCODE" }

$artifact = Join-Path $cmakeBuild "build/$Architecture/Release/AkVirtualCamera.dll"
if (-not (Test-Path -LiteralPath $artifact -PathType Leaf)) {
    throw "CMake build did not produce the expected DirectShow DLL: $artifact"
}
$hash = Get-Sha256Hex -Path $artifact
[pscustomobject]@{
    architecture = $Architecture
    artifact = $artifact
    sha256 = $hash
    network = 'none'
    signed = $false
    release_ready = $false
} | ConvertTo-Json -Compress
