[CmdletBinding()]
param(
    [string] $RepositoryRoot
)

$ErrorActionPreference = 'Stop'
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = Split-Path -Parent $scriptRoot
}
$fixtureRoot = Join-Path $RepositoryRoot 'docs\superpowers\plans\2026-09-04-csharp-rust-sync-fixtures'
$requiredFiles = @(
    'media-effects.json',
    'playback-state.json',
    'audio-candidate.json',
    'rtmp-output.json',
    'virtual-camera.json',
    'douyin-m1.json',
    'error-codes.json'
)

foreach ($fileName in $requiredFiles) {
    $path = Join-Path $fixtureRoot $fileName
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Missing sync fixture: $fileName"
    }

    $document = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    if ($document.schema_version -ne 1 -or [string]::IsNullOrWhiteSpace([string]$document.contract)) {
        throw "Invalid fixture metadata: $fileName"
    }

    $raw = Get-Content -LiteralPath $path -Raw
    if ($raw -match '(?i)(cookie|token|api[_-]?key|stream[_-]?key|secret)') {
        throw "Fixture contains a sensitive field name: $fileName"
    }
}

$virtualCameraPath = Join-Path $fixtureRoot 'virtual-camera.json'
$virtualCamera = Get-Content -LiteralPath $virtualCameraPath -Raw | ConvertFrom-Json
if ($virtualCamera.input.device_name -ne 'GpAutoLive Camera' -or
    $virtualCamera.input.pixel_format -ne 'YUY2' -or
    $virtualCamera.input.width -ne 1280 -or
    $virtualCamera.input.height -ne 720 -or
    $virtualCamera.input.fps -ne 30 -or
    $virtualCamera.input.zero_copy -ne $false) {
    throw 'Virtual camera fixed specification fixture is invalid.'
}

$douyinPath = Join-Path $fixtureRoot 'douyin-m1.json'
$douyin = Get-Content -LiteralPath $douyinPath -Raw | ConvertFrom-Json
if ($douyin.input.event_type -ne 'WebcastChatMessage' -or
    $douyin.expected.model_calls -ne 0 -or
    $douyin.expected.go_calls -ne 0 -or
    $douyin.expected.credential_persisted -ne $false) {
    throw 'Douyin M1 fixture crosses the current scope boundary.'
}

Write-Output "Sync fixtures OK: $($requiredFiles.Count) JSON files"
