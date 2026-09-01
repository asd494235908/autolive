[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [string] $Output,

    [Parameter(Mandatory = $false)]
    [string] $InstallRoot,

    [Parameter(Mandatory = $false)]
    [string] $TestPatternEvidence,

    [Parameter(Mandatory = $false)]
    [string] $DownstreamEvidence,

    [Parameter(Mandatory = $false)]
    [string] $CleanupEvidence
)

$ErrorActionPreference = 'Stop'

function New-Blocker([System.Collections.Generic.List[string]] $Blockers, [string] $Message) {
    $Blockers.Add($Message)
}

function Read-RegistryInstallPath([string] $Path) {
    try {
        $value = Get-ItemProperty -LiteralPath $Path -Name installPath -ErrorAction Stop
        if ($value.installPath -is [string] -and $value.installPath.Trim()) {
            return $value.installPath.TrimEnd('\')
        }
    } catch {
        return $null
    }
    return $null
}

function Find-ExpectedDevice {
    if (-not (Get-Command Get-PnpDevice -ErrorAction SilentlyContinue)) {
        return @()
    }
    @(Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object {
        $_.FriendlyName -eq 'GpAutoLive Camera' -or $_.FriendlyName -eq 'GpAutoLiveCamera'
    })
}

function Find-DirectShowEntry {
    $entries = @()
    if (Get-Command Get-CimInstance -ErrorAction SilentlyContinue) {
        $entries = @(Get-CimInstance Win32_PnPEntity -ErrorAction SilentlyContinue | Where-Object {
            $_.Name -eq 'GpAutoLive Camera' -or $_.Name -eq 'GpAutoLiveCamera'
        } | Select-Object Name, PNPDeviceID, Status)
    }
    return $entries
}

function Invoke-ManagerDevices([string] $Root) {
    $manager = Join-Path $Root 'x64\AkVCamManager.exe'
    if (-not (Test-Path -LiteralPath $manager -PathType Leaf)) {
        return [pscustomobject]@{ path = $manager; present = $false; exitCode = $null; output = '' }
    }
    $output = & $manager devices 2>&1 | Out-String
    return [pscustomobject]@{
        path = $manager
        present = $true
        exitCode = $LASTEXITCODE
        output = $output.Trim()
    }
}

function Read-EvidenceJson([string] $Path, [string] $Label, [System.Collections.Generic.List[string]] $Blockers) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        return $null
    }
    if (-not [IO.Path]::IsPathRooted($Path)) {
        New-Blocker $Blockers "$Label 证据路径必须是绝对路径"
        return $null
    }
    try {
        return Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json
    } catch {
        New-Blocker $Blockers "$Label 证据不是有效 JSON：$Path"
        return $null
    }
}

function Test-StatusPassed([object] $Evidence, [string] $Label, [System.Collections.Generic.List[string]] $Blockers) {
    if (-not $Evidence) {
        New-Blocker $Blockers "$Label 尚未提供"
        return $false
    }
    if ($Evidence.status -ne 'passed') {
        New-Blocker $Blockers "$Label status 不是 passed"
        return $false
    }
    return $true
}

function Write-Report([string] $Path, [object] $Report) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        return
    }
    if (-not [IO.Path]::IsPathRooted($Path)) {
        throw '验收报告输出路径必须是绝对路径'
    }
    $directory = Split-Path -Parent $Path
    if ($directory) {
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
    }
    $temporary = "$Path.$PID.tmp"
    [IO.File]::WriteAllText($temporary, ($Report | ConvertTo-Json -Depth 8) + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporary -Destination $Path -Force
}

$blockers = [System.Collections.Generic.List[string]]::new()
$os = Get-CimInstance Win32_OperatingSystem
$registry64 = Read-RegistryInstallPath 'HKLM:\SOFTWARE\Webcamoid\VirtualCamera'
$registry32 = Read-RegistryInstallPath 'HKLM:\SOFTWARE\WOW6432Node\Webcamoid\VirtualCamera'
$explicitInstallRoot = $null
if ($InstallRoot) {
    if (-not [IO.Path]::IsPathRooted($InstallRoot)) {
        New-Blocker $blockers 'InstallRoot 必须是绝对路径'
    } else {
        $explicitInstallRoot = ([IO.Path]::GetFullPath($InstallRoot)).TrimEnd('\')
    }
}
$root = if ($explicitInstallRoot) { $explicitInstallRoot } elseif ($registry64) { $registry64 } elseif ($registry32) { $registry32 } else { $null }
$devices = @(Find-ExpectedDevice)
$directShow = @(Find-DirectShowEntry)
$manager = if ($root) { Invoke-ManagerDevices $root } else { [pscustomobject]@{ path = $null; present = $false; exitCode = $null; output = '' } }

if (-not $registry64) {
    New-Blocker $blockers '未找到 GpAutoLive AkVirtualCamera x64 注册表所有者；请先安装正式签名组件'
}
if (-not $registry32) {
    New-Blocker $blockers '未找到 GpAutoLive AkVirtualCamera x86 注册表所有者；请先安装正式签名组件'
}
if ($devices.Count -eq 0) {
    New-Blocker $blockers 'PnP 中未找到 GpAutoLive Camera 设备'
}
if ($directShow.Count -eq 0) {
    New-Blocker $blockers 'DirectShow/CIM 中未找到 GpAutoLive Camera 端点'
}
if (-not $manager.present) {
    New-Blocker $blockers 'AkVCamManager.exe 不存在，无法执行设备枚举验收'
} elseif ($manager.exitCode -ne 0) {
    New-Blocker $blockers "AkVCamManager devices 返回退出码 $($manager.exitCode)"
} elseif ($manager.output -notmatch 'GpAutoLiveCamera|GpAutoLive Camera') {
    New-Blocker $blockers 'AkVCamManager devices 输出未包含固定产品设备'
}
if ($registry64 -and $registry32 -and $registry64 -ne $registry32) {
    New-Blocker $blockers 'x86/x64 注册表视图的 installPath 不一致'
}
if ($root) {
    foreach ($relative in @('x64\AkVirtualCamera.dll', 'x86\AkVirtualCamera.dll', 'x64\AkVCamManager.exe')) {
        if (-not (Test-Path -LiteralPath (Join-Path $root $relative) -PathType Leaf)) {
            New-Blocker $blockers "安装根缺少固定组件：$relative"
        }
    }
}

$patternEvidence = Read-EvidenceJson $TestPatternEvidence 'CPU test-pattern' $blockers
$patternPassed = Test-StatusPassed $patternEvidence 'CPU test-pattern' $blockers
if ($patternPassed) {
    foreach ($property in @('format', 'width', 'height', 'fps', 'frames', 'sequenceMonotonic')) {
        if ($null -eq $patternEvidence.$property) {
            New-Blocker $blockers "CPU test-pattern 缺少字段：$property"
        }
    }
    if ($patternEvidence.format -ne 'YUY2' -or $patternEvidence.width -ne 1280 -or
        $patternEvidence.height -ne 720 -or $patternEvidence.fps -ne 30 -or
        $patternEvidence.frames -le 0 -or $patternEvidence.sequenceMonotonic -ne $true) {
        New-Blocker $blockers 'CPU test-pattern 必须证明 YUY2 1280×720@30、帧数大于 0 且序列单调'
    }
}

$downstreamEvidenceValue = Read-EvidenceJson $DownstreamEvidence '下游兼容' $blockers
$downstreamPassed = Test-StatusPassed $downstreamEvidenceValue '下游兼容' $blockers
$requiredDownstream = @('Chrome', 'Edge', 'Teams', 'Zoom', 'Discord', 'OBS', 'Windows Camera', 'DirectShow 32-bit')
if ($downstreamPassed) {
    $downstreamEntries = @($downstreamEvidenceValue.entries)
    foreach ($application in $requiredDownstream) {
        $entry = $downstreamEntries | Where-Object { $_.application -eq $application } | Select-Object -First 1
        if (-not $entry -or $entry.status -ne 'passed') {
            New-Blocker $blockers "下游兼容缺少通过条目：$application"
        }
    }
}

$cleanupEvidenceValue = Read-EvidenceJson $CleanupEvidence '清理验收' $blockers
$cleanupPassed = Test-StatusPassed $cleanupEvidenceValue '清理验收' $blockers
$requiredCleanup = @('退出', '崩溃', '卸载', '客户端占用')
if ($cleanupPassed) {
    $cleanupEntries = @($cleanupEvidenceValue.entries)
    foreach ($scenario in $requiredCleanup) {
        $entry = $cleanupEntries | Where-Object { $_.scenario -eq $scenario } | Select-Object -First 1
        if (-not $entry -or $entry.status -ne 'passed') {
            New-Blocker $blockers "清理验收缺少通过场景：$scenario"
        }
    }
}

$report = [ordered]@{
    schemaVersion = 1
    gate = 'akvirtualcamera-device-compatibility'
    status = if ($blockers.Count -eq 0) { 'passed' } else { 'blocked' }
    reason = '设备验收需要固定设备、CPU test-pattern、下游应用和清理证据全部通过'
    operatingSystem = [ordered]@{ caption = $os.Caption; build = $os.BuildNumber }
    installRoot = $root
    registryOwners = [ordered]@{ x64 = $registry64; x86 = $registry32 }
    pnpDeviceCount = $devices.Count
    directShowEntries = $directShow
    manager = $manager
    testPattern = if ($patternEvidence) { $patternEvidence } else { [ordered]@{ status = 'not_run'; command = $null } }
    downstream = if ($downstreamEvidenceValue) { $downstreamEvidenceValue } else { [ordered]@{ status = 'not_run'; applications = $requiredDownstream } }
    cleanup = if ($cleanupEvidenceValue) { $cleanupEvidenceValue } else { [ordered]@{ status = 'not_run'; scenarios = $requiredCleanup } }
    blockers = @($blockers)
}
Write-Report $Output $report
$report | ConvertTo-Json -Depth 8
if ($report.status -eq 'passed') {
    exit 0
}
exit 2
