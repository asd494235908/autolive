[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',

    [string]$DotnetRoot,

    [string]$OutputPath,

    [ValidateRange(100, 10000)]
    [int]$ResidualGraceMilliseconds = 2000,

    [switch]$PlanOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($PSVersionTable.PSVersion.Major -lt 7) {
    throw 'This acceptance matrix requires PowerShell 7 (pwsh).'
}

$scriptRoot = [IO.Path]::GetFullPath($PSScriptRoot)
$repoRoot = [IO.Path]::GetFullPath((Join-Path $scriptRoot '..'))
$mediaProcessNames = @('mpv', 'ffmpeg', 'ffprobe')
$deferredGates = @(
    [ordered]@{
        id = 'c3_c4_real_media_runtime'
        status = 'not_run_requires_local_fixture'
        reason = '真实 FFprobe、mpv HWND、GPU83/CPU4/Original 和长媒体循环不属于离线合同矩阵。'
    },
    [ordered]@{
        id = 'c4_portaudio_no_input_device'
        status = 'not_run_requires_real_device'
        reason = '无输入设备、拔插、驱动重置和睡眠唤醒必须在真实 PortAudio 环境验证。'
    },
    [ordered]@{
        id = 'c4_portaudio_no_output_device'
        status = 'not_run_requires_real_device'
        reason = '无输出设备、恢复次数和原生调用硬超时必须在真实 PortAudio 环境验证。'
    },
    [ordered]@{
        id = 'c5_rtmp_handshake_and_reconnect'
        status = 'not_run_requires_real_endpoint'
        reason = '有限重连策略已有本地合同测试；Publishing 仍只代表 FFmpeg 进程已启动，握手、断开信号、RTMPS、鉴权和真实重连仍需 ZLMediaKit。'
    }
)
$matrix = @(
    [ordered]@{
        id = 'c3_media_pool'
        phase = 'C3'
        gate = '媒体池原子提交、路径数量和播放身份'
        project = 'tests/GpAutoLive.Core.Tests/GpAutoLive.Core.Tests.csproj'
        filter = 'FullyQualifiedName~MediaPoolServiceTests'
        platform = $null
    },
    [ordered]@{
        id = 'c3_input_probe'
        phase = 'C3'
        gate = '输入路径、FFprobe 参数、超时取消和批次回滚'
        project = 'tests/GpAutoLive.Media.Tests/GpAutoLive.Media.Tests.csproj'
        filter = 'FullyQualifiedName~MediaProbeBoundaryTests|FullyQualifiedName~MediaImportCoordinatorTests'
        platform = 'x64'
    },
    [ordered]@{
        id = 'c4_runtime_and_mpv'
        phase = 'C4'
        gate = '资源清单、mpv 启动计划、IPC 和播放状态边界'
        project = 'tests/GpAutoLive.Media.Tests/GpAutoLive.Media.Tests.csproj'
        filter = 'FullyQualifiedName~RuntimeMediaManifestTests|FullyQualifiedName~MpvLaunchPlanTests|FullyQualifiedName~MpvBoundaryTests|FullyQualifiedName~MpvNamedPipeClientTests|FullyQualifiedName~MpvPlaybackStateMonitorTests|FullyQualifiedName~FfmpegPcmDecodePlanTests'
        platform = 'x64'
    },
    [ordered]@{
        id = 'c4_process_lifecycle'
        phase = 'C4'
        gate = '本机进程超时取消、mpv/FFmpeg 宿主和释放'
        project = 'tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj'
        filter = 'FullyQualifiedName~ExternalProcessBoundaryTests|FullyQualifiedName~WindowsMpvProcessHostTests|FullyQualifiedName~WindowsMpvPlaybackRuntimeTests|FullyQualifiedName~WindowsMpvPlaybackControllerTests|FullyQualifiedName~WindowsFfmpegPcmDecoderTests'
        platform = 'x64'
    },
    [ordered]@{
        id = 'c4_audio_devices'
        phase = 'C4/C5'
        gate = 'PortAudio 资源缺失、非法设备配置、输入输出取消和麦克风门控'
        project = 'tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj'
        filter = 'FullyQualifiedName~WindowsPortAudioDeviceEnumeratorTests|FullyQualifiedName~WindowsPortAudioInputStreamTests|FullyQualifiedName~WindowsPortAudioOutputStreamTests|FullyQualifiedName~WindowsMicrophoneInterludeControllerTests|FullyQualifiedName~WindowsAudioPlaybackControllerTests|FullyQualifiedName~WindowsAudioPauseGateTests'
        platform = 'x64'
    },
    [ordered]@{
        id = 'c5_rtmp_contract'
        phase = 'C5'
        gate = 'RTMP 输入、脱敏和轨道组合合同'
        project = 'tests/GpAutoLive.Contracts.Tests/GpAutoLive.Contracts.Tests.csproj'
        filter = 'FullyQualifiedName~RtmpContractTests'
        platform = $null
    },
    [ordered]@{
        id = 'c5_rtmp_media_plan'
        phase = 'C5'
        gate = 'FFmpeg 参数数组和本机编码器探测降级'
        project = 'tests/GpAutoLive.Media.Tests/GpAutoLive.Media.Tests.csproj'
        filter = 'FullyQualifiedName~RtmpFfmpegCommandBuilderTests|FullyQualifiedName~RtmpEncoderProbeTests'
        platform = 'x64'
    },
    [ordered]@{
        id = 'c5_rtmp_lifecycle'
        phase = 'C5'
        gate = 'RTMP 缺失资源、声音会话、PCM 泵和停止释放'
        project = 'tests/GpAutoLive.Windows.Tests/GpAutoLive.Windows.Tests.csproj'
        filter = 'FullyQualifiedName~WindowsRtmpOutputManagerTests|FullyQualifiedName~WindowsRtmpAudioSessionTests|FullyQualifiedName~WindowsRtmpFinalPcmPumpTests|FullyQualifiedName~WindowsRtmpReconnectCoordinatorTests'
        platform = 'x64'
    }
)

function Resolve-RegularFile([string]$Path, [string]$Name) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "$Name cannot be empty."
    }

    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "$Name must point to an existing file."
    }

    $item = Get-Item -LiteralPath $resolved
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "$Name cannot be a reparse point."
    }

    return $resolved
}

function Get-MediaProcessSnapshot {
    $items = [Collections.Generic.List[object]]::new()
    foreach ($name in $mediaProcessNames) {
        foreach ($process in [Diagnostics.Process]::GetProcessesByName($name)) {
            try {
                $items.Add([ordered]@{
                    name = $name
                    id = [int]$process.Id
                })
            }
            finally {
                $process.Dispose()
            }
        }
    }

    return @($items | Sort-Object name, id)
}

function Get-NewProcesses([object[]]$Before, [object[]]$After) {
    $known = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($item in $Before) {
        $null = $known.Add("$($item.name):$($item.id)")
    }

    return @($After | Where-Object {
        $null -ne $_ -and -not $known.Contains("$($_.name):$($_.id)")
    })
}

function Write-JsonReport([Collections.IDictionary]$Value, [string]$Path) {
    $json = $Value | ConvertTo-Json -Depth 8
    if ([Text.Encoding]::UTF8.GetByteCount($json) -gt 65536) {
        throw 'Acceptance report exceeded the 64 KiB output bound.'
    }

    if (-not [string]::IsNullOrWhiteSpace($Path)) {
        $resolvedOutput = [IO.Path]::GetFullPath($Path)
        $parent = Split-Path -Parent $resolvedOutput
        if ([string]::IsNullOrWhiteSpace($parent)) {
            throw 'OutputPath must include a directory.'
        }

        if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
            New-Item -ItemType Directory -Force -Path $parent | Out-Null
        }

        $parentItem = Get-Item -LiteralPath $parent
        if ($parentItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'OutputPath parent cannot be a reparse point.'
        }

        if (Test-Path -LiteralPath $resolvedOutput) {
            $outputItem = Get-Item -LiteralPath $resolvedOutput
            if ($outputItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw 'OutputPath cannot be a reparse point.'
            }
        }

        $temporary = "$resolvedOutput.tmp-$([Guid]::NewGuid().ToString('N'))"
        try {
            [IO.File]::WriteAllText($temporary, $json, [Text.UTF8Encoding]::new($false))
            [IO.File]::Move($temporary, $resolvedOutput, $true)
        }
        finally {
            if (Test-Path -LiteralPath $temporary -PathType Leaf) {
                Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
            }
        }
    }

    Write-Output $json
}

function Invoke-MatrixRow([string]$DotnetExecutable, [Collections.IDictionary]$Row) {
    $projectPath = Resolve-RegularFile (Join-Path $repoRoot $Row.project) 'matrix project'
    $arguments = @(
        'test',
        $projectPath,
        '-c', $Configuration,
        '--no-restore',
        '--no-build',
        '--filter', $Row.filter,
        '--logger', 'console;verbosity=minimal'
    )
    if ($null -ne $Row.platform) {
        $arguments += "-p:Platform=$($Row.platform)"
    }
    $tail = [Collections.Generic.Queue[string]]::new()
    $stopwatch = [Diagnostics.Stopwatch]::StartNew()

    Write-Host "[$($Row.id)] $($Row.gate)"
    & $DotnetExecutable @arguments 2>&1 | ForEach-Object {
        $line = $_.ToString()
        Write-Host $line
        if ($line.Length -gt 512) {
            $line = $line.Substring(0, 512)
        }
        if ($tail.Count -ge 12) {
            $null = $tail.Dequeue()
        }
        $tail.Enqueue($line)
    }
    $exitCode = $LASTEXITCODE
    $stopwatch.Stop()

    return [ordered]@{
        id = $Row.id
        phase = $Row.phase
        gate = $Row.gate
        project = $Row.project
        filter = $Row.filter
        status = if ($exitCode -eq 0) { 'passed' } else { 'failed' }
        exit_code = $exitCode
        duration_milliseconds = [int64]$stopwatch.ElapsedMilliseconds
        output_tail = @($tail.ToArray())
    }
}

if (-not [OperatingSystem]::IsWindows()) {
    throw 'This local acceptance matrix only supports Windows.'
}

$report = [ordered]@{
    schema_version = 1
    captured_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
    mode = if ($PlanOnly) { 'plan_only' } else { 'offline_contract' }
    status = 'not_started'
    configuration = $Configuration
    network_policy = 'no_restore_no_build_real_fixtures_disabled'
    matrix = @()
    deferred_gates = @($deferredGates)
    preexisting_media_processes = @()
    residual_media_processes = @()
}

if ($PlanOnly) {
    $report.status = 'plan_only'
    $report.matrix = @($matrix | ForEach-Object {
        [ordered]@{
            id = $_.id
            phase = $_.phase
            gate = $_.gate
            project = $_.project
            filter = $_.filter
            status = 'planned'
        }
    })
    Write-JsonReport $report $null
    exit 0
}

if ([string]::IsNullOrWhiteSpace($DotnetRoot)) {
    $DotnetRoot = Join-Path $repoRoot '.tools\dotnet'
}
$dotnet = Resolve-RegularFile (Join-Path ([IO.Path]::GetFullPath($DotnetRoot)) 'dotnet.exe') 'DotnetRoot\dotnet.exe'
$fixtureEnvironment = @(Get-ChildItem Env: | Where-Object { $_.Name -like 'AUTOLIVE_TEST_*' })
$before = @(Get-MediaProcessSnapshot)
$report.preexisting_media_processes = @($before)
$results = [Collections.Generic.List[object]]::new()

try {
    foreach ($variable in $fixtureEnvironment) {
        Remove-Item -LiteralPath "Env:$($variable.Name)"
    }
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
    $env:DOTNET_NOLOGO = '1'
    $env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = '1'
    $env:DOTNET_MULTILEVEL_LOOKUP = '0'
    $env:NUGET_XMLDOC_MODE = 'skip'

    foreach ($row in $matrix) {
        $results.Add((Invoke-MatrixRow $dotnet $row))
    }
}
finally {
    foreach ($variable in $fixtureEnvironment) {
        Set-Item -LiteralPath "Env:$($variable.Name)" -Value $variable.Value
    }
}

$report.matrix = @($results)
$residualWatch = [Diagnostics.Stopwatch]::StartNew()
do {
    $after = @(Get-MediaProcessSnapshot)
    $residual = @(Get-NewProcesses $before $after)
    if ($residual.Count -eq 0 -or $residualWatch.ElapsedMilliseconds -ge $ResidualGraceMilliseconds) {
        break
    }
    Start-Sleep -Milliseconds 100
} while ($true)

$report.residual_media_processes = @($residual)
$failedRows = @($results | Where-Object { $_.status -ne 'passed' })
$report.status = if ($failedRows.Count -eq 0 -and $residual.Count -eq 0) { 'passed' } else { 'failed' }
Write-JsonReport $report $OutputPath
if ($report.status -eq 'passed') {
    exit 0
}
exit 1
