$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$inputRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\build-inputs')).Path
$toolchainRoot = Join-Path $inputRoot 'toolchain'
$sdkVersion = '10.0.26100.0'
$msvcVersion = '14.44.35207'
$sdkRoot = 'C:\Program Files (x86)\Windows Kits\10'
$msvcRoot = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\$msvcVersion"
$stageRoot = Join-Path $inputRoot ('.stage-msvc-' + [guid]::NewGuid().ToString('N'))

function Copy-DirectoryContents([string] $Source, [string] $Destination) {
    if (-not (Test-Path -LiteralPath $Source -PathType Container)) {
        throw "缺少宿主工具链目录：$Source"
    }
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    Get-ChildItem -LiteralPath $Source -Force | Copy-Item -Recurse -Force -Destination $Destination
}

function New-ToolchainArchive([string] $Source, [string] $Name) {
    $destination = Join-Path $toolchainRoot $Name
    & tar.exe -czf $destination -C $Source .
    if ($LASTEXITCODE -ne 0) {
        throw "生成工具链归档失败：$Name"
    }
}

New-Item -ItemType Directory -Force -Path $toolchainRoot, $stageRoot | Out-Null
try {
    $windowsStage = Join-Path $stageRoot 'windows-sdk'
    $ucrtStage = Join-Path $stageRoot 'ucrt'
    $msvcStage = Join-Path $stageRoot 'msvc-toolset'
    foreach ($name in @('shared', 'um', 'winrt', 'cppwinrt')) {
        Copy-DirectoryContents `
            (Join-Path $sdkRoot "Include\$sdkVersion\$name") `
            (Join-Path $windowsStage "include\$name")
    }
    Copy-DirectoryContents `
        (Join-Path $sdkRoot "Lib\$sdkVersion\um\x64") `
        (Join-Path $windowsStage 'lib\um\x64')
    Copy-DirectoryContents `
        (Join-Path $sdkRoot "Include\$sdkVersion\ucrt") `
        (Join-Path $ucrtStage 'include')
    Copy-DirectoryContents `
        (Join-Path $sdkRoot "Lib\$sdkVersion\ucrt\x64") `
        (Join-Path $ucrtStage 'lib\x64')
    Copy-DirectoryContents (Join-Path $msvcRoot 'include') (Join-Path $msvcStage 'include')
    Copy-DirectoryContents (Join-Path $msvcRoot 'lib\x64') (Join-Path $msvcStage 'lib\x64')

    New-ToolchainArchive $windowsStage "windows-sdk-$sdkVersion.tar.gz"
    New-ToolchainArchive $ucrtStage "ucrt-$sdkVersion.tar.gz"
    New-ToolchainArchive $msvcStage "msvc-toolset-$msvcVersion.tar.gz"
} finally {
    if (Test-Path -LiteralPath $stageRoot) {
        $resolvedStage = (Resolve-Path -LiteralPath $stageRoot).Path
        $relativeStage = [System.IO.Path]::GetRelativePath($inputRoot, $resolvedStage)
        if ($relativeStage -notmatch '^\.stage-msvc-[a-f0-9]{32}$') {
            throw "拒绝清理未验证暂存目录：$resolvedStage"
        }
        Remove-Item -Recurse -Force -LiteralPath $resolvedStage
    }
}

Get-ChildItem -LiteralPath $toolchainRoot -File |
    Where-Object Name -In @(
        "windows-sdk-$sdkVersion.tar.gz",
        "ucrt-$sdkVersion.tar.gz",
        "msvc-toolset-$msvcVersion.tar.gz"
    ) |
    Sort-Object Name |
    Select-Object Name, Length
