[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ExecutablePath,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath,

    [ValidateRange(1, 3600)]
    [int]$DurationSeconds = 30,

    [ValidateRange(100, 60000)]
    [int]$IntervalMilliseconds = 500,

    [ValidateRange(0, 60000)]
    [int]$WarmupMilliseconds = 2000,

    [string]$RuntimeRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class GpAutoLiveGuiResourceProbe
{
    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetGuiResources(IntPtr processHandle, uint flags);
}
'@

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

function Resolve-Directory([string]$Path, [string]$Name) {
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

function Get-GuiResourceCount([Diagnostics.Process]$Process, [uint32]$Flags) {
    try {
        $count = [GpAutoLiveGuiResourceProbe]::GetGuiResources($Process.Handle, $Flags)
        if ($count -eq 0) {
            return $null
        }

        return [int64]$count
    }
    catch [InvalidOperationException] {
        return $null
    }
    catch [ComponentModel.Win32Exception] {
        return $null
    }
}

if (-not [OperatingSystem]::IsWindows()) {
    throw 'This baseline collector only supports Windows.'
}

$executable = Resolve-RegularFile $ExecutablePath 'ExecutablePath'
$output = [IO.Path]::GetFullPath($OutputPath)
$outputParent = Split-Path -Parent $output
if ([string]::IsNullOrWhiteSpace($outputParent)) {
    throw 'OutputPath must include a directory.'
}

if (-not (Test-Path -LiteralPath $outputParent -PathType Container)) {
    New-Item -ItemType Directory -Force -Path $outputParent | Out-Null
}

$runtime = $null
if (-not [string]::IsNullOrWhiteSpace($RuntimeRoot)) {
    $runtime = Resolve-Directory $RuntimeRoot 'RuntimeRoot'
}

$logicalProcessors = [Environment]::ProcessorCount
$process = [Diagnostics.Process]::new()
$process.StartInfo = [Diagnostics.ProcessStartInfo]::new()
$process.StartInfo.FileName = $executable
$process.StartInfo.WorkingDirectory = Split-Path -Parent $executable
$process.StartInfo.UseShellExecute = $false
$process.StartInfo.CreateNoWindow = $true
if ($null -ne $runtime) {
    $process.StartInfo.Environment['DOTNET_ROOT'] = $runtime
    $process.StartInfo.Environment['DOTNET_ROOT_X64'] = $runtime
}

$samples = [Collections.Generic.List[object]]::new()
$startedAt = [DateTimeOffset]::UtcNow
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
$previousCpu = $null
$previousTimestamp = $null
$status = 'not_started'
$started = $false

try {
    if (-not $process.Start()) {
        throw 'Target process did not start.'
    }

    $started = $true
    $status = 'running'
    if ($WarmupMilliseconds -gt 0) {
        Start-Sleep -Milliseconds $WarmupMilliseconds
    }

    while ($stopwatch.Elapsed.TotalSeconds -lt $DurationSeconds) {
        if ($process.HasExited) {
            $status = 'exited'
            break
        }

        try {
            $process.Refresh()
            $timestamp = $stopwatch.Elapsed.TotalSeconds
            $totalCpu = $process.TotalProcessorTime.TotalSeconds
            $cpuPercent = $null
            if ($null -ne $previousCpu -and $null -ne $previousTimestamp) {
                $elapsed = $timestamp - $previousTimestamp
                if ($elapsed -gt 0 -and $totalCpu -ge $previousCpu) {
                    $cpuPercent = [Math]::Min(
                        100.0,
                        [Math]::Max(0.0, (($totalCpu - $previousCpu) / ($elapsed * $logicalProcessors)) * 100.0))
                }
            }

            $samples.Add([ordered]@{
                captured_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
                elapsed_ms = [Math]::Round($timestamp * 1000.0, 0)
                private_working_set_bytes = [int64]$process.PrivateMemorySize64
                working_set_bytes = [int64]$process.WorkingSet64
                thread_count = [int]$process.Threads.Count
                handle_count = [int]$process.HandleCount
                gdi_object_count = Get-GuiResourceCount $process 0
                user_object_count = Get-GuiResourceCount $process 1
                cpu_percent = $cpuPercent
            })
            $previousCpu = $totalCpu
            $previousTimestamp = $timestamp
        }
        catch [InvalidOperationException] {
            $status = 'exited'
            break
        }
        catch [ComponentModel.Win32Exception] {
            $status = 'unavailable'
            break
        }

        Start-Sleep -Milliseconds $IntervalMilliseconds
    }

    if ($status -eq 'running') {
        $status = 'completed'
    }
}
finally {
    if ($started -and $process.HasExited -eq $false) {
        try {
            $process.CloseMainWindow() | Out-Null
        }
        catch [InvalidOperationException] {
            # The target may have exited between the check and the close request.
        }

        if (-not $process.WaitForExit(4000)) {
            try {
                $process.Kill($true)
                $process.WaitForExit(2000)
            }
            catch [InvalidOperationException] {
                # The target may have exited while the bounded cleanup was running.
            }
            catch [ComponentModel.Win32Exception] {
                # Cleanup failure is recorded by the process state, not echoed as raw text.
            }
        }
    }

    $process.Dispose()
}

$privateValues = @($samples | ForEach-Object { $_.private_working_set_bytes })
$workingValues = @($samples | ForEach-Object { $_.working_set_bytes })
$gdiValues = @($samples | Where-Object { $null -ne $_.gdi_object_count } | ForEach-Object { $_.gdi_object_count })
$userValues = @($samples | Where-Object { $null -ne $_.user_object_count } | ForEach-Object { $_.user_object_count })
$cpuValues = @($samples | Where-Object { $null -ne $_.cpu_percent } | ForEach-Object { $_.cpu_percent })
$summary = [ordered]@{
    sample_count = $samples.Count
    private_working_set_min_bytes = if ($privateValues.Count) { ($privateValues | Measure-Object -Minimum).Minimum } else { $null }
    private_working_set_max_bytes = if ($privateValues.Count) { ($privateValues | Measure-Object -Maximum).Maximum } else { $null }
    working_set_min_bytes = if ($workingValues.Count) { ($workingValues | Measure-Object -Minimum).Minimum } else { $null }
    working_set_max_bytes = if ($workingValues.Count) { ($workingValues | Measure-Object -Maximum).Maximum } else { $null }
    gdi_object_count_min = if ($gdiValues.Count) { ($gdiValues | Measure-Object -Minimum).Minimum } else { $null }
    gdi_object_count_max = if ($gdiValues.Count) { ($gdiValues | Measure-Object -Maximum).Maximum } else { $null }
    user_object_count_min = if ($userValues.Count) { ($userValues | Measure-Object -Minimum).Minimum } else { $null }
    user_object_count_max = if ($userValues.Count) { ($userValues | Measure-Object -Maximum).Maximum } else { $null }
    cpu_percent_max = if ($cpuValues.Count) { [Math]::Round(($cpuValues | Measure-Object -Maximum).Maximum, 2) } else { $null }
}

$report = [ordered]@{
    schema_version = 1
    captured_at_utc = $startedAt.ToString('O')
    platform = 'windows'
    executable = [IO.Path]::GetFileName($executable)
    duration_seconds = $DurationSeconds
    interval_milliseconds = $IntervalMilliseconds
    warmup_milliseconds = $WarmupMilliseconds
    logical_processor_count = $logicalProcessors
    status = $status
    summary = $summary
    samples = $samples
}

$json = $report | ConvertTo-Json -Depth 6
[IO.File]::WriteAllText($output, $json, [Text.UTF8Encoding]::new($false))
Write-Output "Baseline written: $output"
