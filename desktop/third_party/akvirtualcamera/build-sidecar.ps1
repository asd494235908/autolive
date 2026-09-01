[CmdletBinding()]
param(
    [ValidateSet('x64')]
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
    $BuildRoot = Join-Path $componentRoot 'local-build'
}

$cmake = Get-Command cmake -ErrorAction Stop
$tar = Get-Command tar -ErrorAction Stop
$git = Get-Command git -ErrorAction Stop
if (-not (Test-Path -LiteralPath $SourceArchive -PathType Leaf)) {
    throw "AkVirtualCamera source archive not found: $SourceArchive"
}

$sourceStage = Join-Path $BuildRoot "source-$Architecture"
$cmakeBuild = Join-Path $BuildRoot "cmake-$Architecture"
$installRoot = Join-Path $BuildRoot "install-$Architecture"
New-Item -ItemType Directory -Path $sourceStage,$cmakeBuild,$installRoot -Force | Out-Null

& $tar.Source -xf $SourceArchive -C $sourceStage
$sourceDir = Get-ChildItem -LiteralPath $sourceStage -Directory | Select-Object -First 1
if (-not $sourceDir) {
    throw 'AkVirtualCamera source archive did not contain a top-level directory'
}

# The upstream service socket is patched to loopback before CMake sees the tree.
# Keep the patch application deterministic and fail closed if Git is unavailable;
# re-running with the same BuildRoot verifies an already applied patch.
$patchFile = Join-Path $componentRoot 'patches/0002-loopback-service-socket.patch'
$repositoryRoot = (Resolve-Path (Join-Path $componentRoot '..\..\..')).Path
Apply-LoopbackPatch -SourceDirectory $sourceDir.FullName -PatchFile $patchFile `
    -RepositoryRoot $repositoryRoot -GitPath $git.Source

$generator = $env:CMAKE_GENERATOR
if ([string]::IsNullOrWhiteSpace($generator)) {
    throw 'CMAKE_GENERATOR must select the installed MSVC generator (for example Visual Studio 17 2022)'
}

$includeFile = Join-Path $componentRoot 'cmake/enable-gpautolive-sidecar.cmake'
$sidecarSource = Join-Path $componentRoot 'sidecar'
& $cmake.Source -S $sourceDir.FullName -B $cmakeBuild -G $generator -A $Architecture `
    "-DCMAKE_BUILD_TYPE=Release" `
    "-DCMAKE_INSTALL_PREFIX=$installRoot" `
    '-DAKVCAM_BUILD_GPAUTOLIVE_SIDECAR=ON' `
    "-DAKVCAM_GPAUTOLIVE_SIDECAR_SOURCE=$sidecarSource" `
    "-DCMAKE_PROJECT_INCLUDE=$includeFile"
if ($LASTEXITCODE -ne 0) { throw "CMake configure failed: $LASTEXITCODE" }

& $cmake.Source --build $cmakeBuild --config Release --parallel
if ($LASTEXITCODE -ne 0) { throw "CMake build failed: $LASTEXITCODE" }
& $cmake.Source --install $cmakeBuild --config Release
if ($LASTEXITCODE -ne 0) { throw "CMake install failed: $LASTEXITCODE" }

$artifact = Join-Path $installRoot 'bin/akvirtualcamera-sidecar-x64.exe'
if (-not (Test-Path -LiteralPath $artifact -PathType Leaf)) {
    throw "CMake install did not produce the expected sidecar: $artifact"
}
$capi = Join-Path $installRoot 'x64/vcam_capi.dll'
if (-not (Test-Path -LiteralPath $capi -PathType Leaf)) {
    throw "CMake install did not produce the expected C API: $capi"
}
# The sidecar loads the C API only from its application directory.
# Keep both files in the same controlled output directory; no system install is performed.
$capiSidecarPath = Join-Path -Path (Split-Path -Parent $artifact) -ChildPath 'vcam_capi.dll'
Copy-Item -LiteralPath $capi -Destination $capiSidecarPath -Force

$hash = Get-Sha256Hex -Path $artifact
$capiHash = Get-Sha256Hex -Path $capiSidecarPath
[pscustomobject]@{
    architecture = $Architecture
    artifact = $artifact
    sha256 = $hash
    capi_artifact = $capiSidecarPath
    capi_sha256 = $capiHash
    network = 'none'
    signed = $false
    release_ready = $false
} | ConvertTo-Json -Compress
