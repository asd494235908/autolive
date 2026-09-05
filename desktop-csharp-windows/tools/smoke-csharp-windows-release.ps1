[CmdletBinding()]
param(
    [string]$PackageRoot,

    [string]$ExecutablePath,

    [string]$RuntimeRoot,

    [ValidateRange(1000, 120000)]
    [int]$StartupTimeoutMilliseconds = 10000,

    [ValidateRange(1000, 30000)]
    [int]$ShutdownTimeoutMilliseconds = 5000,

    [ValidateRange(0, 10000)]
    [int]$ReadyHoldMilliseconds = 500,

    [ValidateRange(100, 10000)]
    [int]$ResidualGraceMilliseconds = 2000,

    [string]$OutputPath,

    [switch]$PlanOnly,

    [switch]$WhatIf
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$processNames = @('GpAutoLive', 'mpv', 'ffmpeg', 'ffprobe')
$scriptRoot = [IO.Path]::GetFullPath($PSScriptRoot)
$repoRoot = [IO.Path]::GetFullPath((Join-Path $scriptRoot '..'))
$startedAt = [DateTimeOffset]::UtcNow
$report = [ordered]@{
    schema_version = 1
    captured_at_utc = $startedAt.ToString('O')
    platform = 'windows'
    mode = if ($PlanOnly -or $WhatIf) { 'plan_only' } else { 'smoke' }
    status = 'not_started'
    executable = $null
    runtime_root = $null
    runtime_executable = $null
    startup_timeout_milliseconds = $StartupTimeoutMilliseconds
    shutdown_timeout_milliseconds = $ShutdownTimeoutMilliseconds
    ready_hold_milliseconds = $ReadyHoldMilliseconds
    residual_grace_milliseconds = $ResidualGraceMilliseconds
    process_id = $null
    wait_for_input_idle = $null
    window_available = $false
    window_title = $null
    close_main_window = $null
    exited = $false
    forced_kill = $false
    exit_code = $null
    residual_process_count = 0
    residual_processes = @()
    preexisting_processes = @()
    failure_code = $null
    message = $null
    duration_milliseconds = $null
}

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

function Resolve-RegularDirectory([string]$Path, [string]$Name) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "$Name cannot be empty."
    }

    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
        throw "$Name must point to an existing directory."
    }

    $item = Get-Item -LiteralPath $resolved
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "$Name cannot be a reparse point."
    }

    return $resolved.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
}

function Assert-PathInside([string]$ChildPath, [string]$ParentPath, [string]$Name) {
    $parentWithSeparator = $ParentPath.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
    if (-not $ChildPath.StartsWith($parentWithSeparator, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Name must be inside PackageRoot."
    }
}

function Get-ProcessSnapshot {
    $items = [Collections.Generic.List[object]]::new()
    $errors = [Collections.Generic.List[string]]::new()

    foreach ($name in $processNames) {
        try {
            $matches = [Diagnostics.Process]::GetProcessesByName($name)
            foreach ($match in $matches) {
                try {
                    $items.Add([ordered]@{
                        name = $name
                        id = [int]$match.Id
                    })
                }
                finally {
                    $match.Dispose()
                }
            }
        }
        catch [ComponentModel.Win32Exception] {
            $errors.Add("process_probe_$name")
        }
        catch [InvalidOperationException] {
            $errors.Add("process_probe_$name")
        }
    }

    return [ordered]@{
        items = @($items)
        errors = @($errors)
    }
}

function Wait-WindowAvailable([Diagnostics.Process]$Process, [int]$TimeoutMilliseconds) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt $TimeoutMilliseconds) {
        if ($Process.HasExited) {
            return [ordered]@{
                available = $false
                title = $null
            }
        }

        try {
            $Process.Refresh()
            if ($Process.MainWindowHandle -ne [IntPtr]::Zero) {
                return [ordered]@{
                    available = $true
                    title = $Process.MainWindowTitle
                }
            }
        }
        catch [InvalidOperationException] {
            return [ordered]@{
                available = $false
                title = $null
            }
        }

        Start-Sleep -Milliseconds 100
    }

    return [ordered]@{
        available = $false
        title = $null
    }
}

function Write-Report([Collections.IDictionary]$Value, [string]$Path) {
    $json = $Value | ConvertTo-Json -Depth 8 -Compress
    if ($json.Length -gt 65536) {
        throw 'Smoke report exceeded the 64 KiB output bound.'
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

        if (Test-Path -LiteralPath $resolvedOutput -PathType Leaf) {
            $outputItem = Get-Item -LiteralPath $resolvedOutput
            if ($outputItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw 'OutputPath cannot be a reparse point.'
            }
        }

        $temporary = "$resolvedOutput.tmp-$([Guid]::NewGuid().ToString('N'))"
        try {
            [IO.File]::WriteAllText($temporary, $json, [Text.UTF8Encoding]::new($false))
            if (Test-Path -LiteralPath $resolvedOutput -PathType Leaf) {
                [IO.File]::Move($temporary, $resolvedOutput, $true)
            }
            else {
                [IO.File]::Move($temporary, $resolvedOutput)
            }
        }
        finally {
            if (Test-Path -LiteralPath $temporary -PathType Leaf) {
                Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
            }
        }
    }

    Write-Output $json
}

function Set-Failure([string]$Code, [string]$Message) {
    $report.status = 'failed'
    $report.failure_code = $Code
    $report.message = $Message
}

if (-not [OperatingSystem]::IsWindows()) {
    Set-Failure 'not_windows' 'This release smoke script only supports Windows.'
    Write-Report $report $null
    exit 1
}

$process = $null
$startedByScript = $false
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
$exitStatus = 1

try {
    if ([string]::IsNullOrWhiteSpace($PackageRoot) -and [string]::IsNullOrWhiteSpace($ExecutablePath)) {
        throw 'Specify PackageRoot or ExecutablePath.'
    }

    $package = $null
    if (-not [string]::IsNullOrWhiteSpace($PackageRoot)) {
        $package = Resolve-RegularDirectory $PackageRoot 'PackageRoot'
    }

    if ([string]::IsNullOrWhiteSpace($ExecutablePath)) {
        $ExecutablePath = Join-Path $package 'GpAutoLive.exe'
    }

    $executable = Resolve-RegularFile $ExecutablePath 'ExecutablePath'
    if (-not ([IO.Path]::GetFileName($executable)).Equals('GpAutoLive.exe', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'ExecutablePath must name GpAutoLive.exe.'
    }

    if ($null -ne $package) {
        Assert-PathInside $executable $package 'ExecutablePath'
    }

    if ([string]::IsNullOrWhiteSpace($RuntimeRoot)) {
        $RuntimeRoot = Join-Path $repoRoot '.tools\dotnet'
    }

    $runtime = Resolve-RegularDirectory $RuntimeRoot 'RuntimeRoot'
    $runtimeExecutable = Resolve-RegularFile (Join-Path $runtime 'dotnet.exe') 'RuntimeRoot\dotnet.exe'
    $report.executable = $executable
    $report.runtime_root = $runtime
    $report.runtime_executable = $runtimeExecutable

    if ($PlanOnly -or $WhatIf) {
        $report.status = 'plan_only'
        $report.message = 'No process was started and no report file was written in plan mode.'
        $report.duration_milliseconds = [int64]$stopwatch.ElapsedMilliseconds
        Write-Report $report $null
        exit 0
    }

    $before = Get-ProcessSnapshot
    if ($before.errors.Count -gt 0) {
        throw 'Unable to determine whether a conflicting process is already running.'
    }

    $report.preexisting_processes = @($before.items)
    if ($before.items.Count -gt 0) {
        $report.status = 'blocked_preexisting_processes'
        $report.failure_code = 'preexisting_processes'
        $report.message = 'A protected process name was already running; the smoke test did not start or terminate it.'
        $report.duration_milliseconds = [int64]$stopwatch.ElapsedMilliseconds
        Write-Report $report $OutputPath
        exit 2
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = [Diagnostics.ProcessStartInfo]::new()
    $process.StartInfo.FileName = $executable
    $process.StartInfo.WorkingDirectory = Split-Path -Parent $executable
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Normal
    $process.StartInfo.Environment['DOTNET_ROOT'] = $runtime
    $process.StartInfo.Environment['DOTNET_ROOT_X64'] = $runtime
    $process.StartInfo.Environment['DOTNET_MULTILEVEL_LOOKUP'] = '0'

    if (-not $process.Start()) {
        throw 'Target process did not start.'
    }

    $startedByScript = $true
    $report.status = 'starting'
    $report.process_id = [int]$process.Id

    $idle = $false
    try {
        $idle = $process.WaitForInputIdle($StartupTimeoutMilliseconds)
    }
    catch [InvalidOperationException] {
        $report.wait_for_input_idle = $false
    }
    catch [ComponentModel.Win32Exception] {
        $report.wait_for_input_idle = $false
    }
    if ($null -eq $report.wait_for_input_idle) {
        $report.wait_for_input_idle = [bool]$idle
    }

    $window = Wait-WindowAvailable $process $StartupTimeoutMilliseconds
    $report.window_available = [bool]$window.available
    $report.window_title = $window.title
    if (-not $window.available) {
        Set-Failure 'window_not_available' 'The process did not expose a top-level window within the startup budget.'
    }
    else {
        $report.status = 'ready'
        if ($ReadyHoldMilliseconds -gt 0) {
            Start-Sleep -Milliseconds $ReadyHoldMilliseconds
        }
    }
}
catch [ComponentModel.Win32Exception] {
    Set-Failure 'process_start_failed' 'Windows refused to start or inspect the target process.'
}
catch [InvalidOperationException] {
    Set-Failure 'process_state_failed' 'The target process changed state before the smoke check completed.'
}
catch {
    Set-Failure 'validation_failed' $_.Exception.Message
}
finally {
    if ($null -ne $process) {
        try {
            if (-not $process.HasExited) {
                try {
                    $report.close_main_window = [bool]$process.CloseMainWindow()
                }
                catch [InvalidOperationException] {
                    $report.close_main_window = $false
                }

                if ($process.WaitForExit($ShutdownTimeoutMilliseconds)) {
                    $report.exited = $true
                }
                else {
                    $report.forced_kill = $true
                    try {
                        $process.Kill($true)
                        $report.exited = $process.WaitForExit($ShutdownTimeoutMilliseconds)
                    }
                    catch [InvalidOperationException] {
                        $report.exited = $process.HasExited
                    }
                    catch [ComponentModel.Win32Exception] {
                        $report.exited = $false
                    }
                }
            }
            else {
                $report.exited = $true
            }

            if ($report.exited) {
                try {
                    $report.exit_code = [int]$process.ExitCode
                }
                catch [InvalidOperationException] {
                    $report.exit_code = $null
                }
            }
        }
        finally {
            $process.Dispose()
        }
    }
}

if ($report.mode -eq 'smoke' -and $startedByScript) {
    $remaining = $null
    $residualWatch = [Diagnostics.Stopwatch]::StartNew()
    do {
        $snapshot = Get-ProcessSnapshot
        if ($snapshot.errors.Count -gt 0) {
            $report.residual_processes = @()
            $report.residual_process_count = -1
            Set-Failure 'residual_probe_failed' 'Unable to determine whether protected processes remain.'
            break
        }

        $remaining = @($snapshot.items)
        if ($remaining.Count -eq 0) {
            break
        }

        if ($residualWatch.ElapsedMilliseconds -lt $ResidualGraceMilliseconds) {
            Start-Sleep -Milliseconds 100
        }
    } while ($residualWatch.ElapsedMilliseconds -lt $ResidualGraceMilliseconds)

    if ($null -ne $remaining -and $remaining.Count -gt 0) {
        $report.residual_process_count = $remaining.Count
        $report.residual_processes = @($remaining | Select-Object -First 32)
        Set-Failure 'residual_processes' 'Protected process names remained after shutdown.'
    }

    if ($report.status -in @('ready', 'starting') -and
        $report.wait_for_input_idle -eq $true -and
        $report.window_available -eq $true -and
        $report.close_main_window -eq $true -and
        $report.exited -and
        -not $report.forced_kill -and
        $report.exit_code -eq 0 -and
        $report.residual_process_count -eq 0) {
        $report.status = 'passed'
        $report.message = 'Startup, window readiness, graceful shutdown and residual-process checks passed.'
        $exitStatus = 0
    }
    elseif ($report.status -ne 'failed') {
        $report.status = 'failed'
        if ([string]::IsNullOrWhiteSpace($report.failure_code)) {
            $report.failure_code = 'shutdown_failed'
        }
    }
}

$report.duration_milliseconds = [int64]$stopwatch.ElapsedMilliseconds
Write-Report $report $OutputPath
exit $exitStatus
