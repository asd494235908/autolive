[CmdletBinding()]
param([string]$PackageRoot)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $PackageRoot) { $PackageRoot = Join-Path $PSScriptRoot '../artifacts/virtual-camera-development' }
$component = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../desktop/third_party/akvirtualcamera'))
$PackageRoot = [IO.Path]::GetFullPath($PackageRoot)
$lock = Get-Content -LiteralPath (Join-Path $component 'upstream.lock.json') -Raw | ConvertFrom-Json
$archive = Join-Path $component ('source/akvirtualcamera-' + $lock.commit + '.tar.gz')
if ((Get-Item -LiteralPath $archive).Length -ne $lock.source_archive.size_bytes -or
    (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $lock.source_archive.sha256) { throw 'Locked source hash/size mismatch.' }
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vs) { throw 'Install local Visual Studio C++ Build Tools first.' }
    $env:Path = (Join-Path $vs 'Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin') + ';' + $env:Path
}
if (-not $env:CMAKE_GENERATOR) { $env:CMAKE_GENERATOR = 'Visual Studio 17 2022' }
& (Join-Path $component 'build-sidecar.ps1')
& (Join-Path $component 'build-directshow.ps1') -Architecture x86
$root = Join-Path $PackageRoot 'akvirtualcamera'
New-Item -ItemType Directory -Path (Join-Path $root 'bin'),(Join-Path $root 'x64'),(Join-Path $root 'x86') -Force | Out-Null
$inputs = [ordered]@{
    'bin/akvirtualcamera-sidecar-x64.exe' = 'local-build/install-x64/bin/akvirtualcamera-sidecar-x64.exe'
    'bin/vcam_capi.dll' = 'local-build/install-x64/bin/vcam_capi.dll'
    'x64/AkVirtualCamera.dll' = 'local-build/cmake-x64/build/x64/Release/AkVirtualCamera.dll'
    'x64/AkVCamAssistant.exe' = 'local-build/install-x64/x64/AkVCamAssistant.exe'
    'x64/AkVCamManager.exe' = 'local-build/install-x64/x64/AkVCamManager.exe'
    'x86/AkVirtualCamera.dll' = 'local-build-directshow/cmake-x86/build/x86/Release/AkVirtualCamera.dll'
}
$files = [ordered]@{}
foreach ($entry in $inputs.GetEnumerator()) {
    $destination = Join-Path $root $entry.Key
    Copy-Item -LiteralPath (Join-Path $component $entry.Value) -Destination $destination -Force
    $files[$entry.Key] = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
}
foreach ($name in @('COPYING','NOTICE.md','MODIFICATIONS.md','corresponding-source-manifest.json')) {
    Copy-Item -LiteralPath (Join-Path $component $name) -Destination (Join-Path $root $name) -Force
}
$manifestJson = @{schemaVersion=1;files=$files} | ConvertTo-Json -Depth 4
[IO.File]::WriteAllText((Join-Path $root 'development-manifest.json'),$manifestJson,[Text.UTF8Encoding]::new($false))
Write-Output "Unsigned local development package: $PackageRoot (not release ready)"
