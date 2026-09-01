[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $SidecarPath,

    [Parameter(Mandatory = $false)]
    [ValidateRange(1, 60)]
    [int] $Seconds = 1,

    [Parameter(Mandatory = $false)]
    [string] $Output
)

$ErrorActionPreference = 'Stop'

function Require-AbsolutePath([string] $Path, [string] $Label) {
    if ([string]::IsNullOrWhiteSpace($Path) -or -not [IO.Path]::IsPathRooted($Path)) {
        throw "$Label 必须是绝对路径"
    }
    return [IO.Path]::GetFullPath($Path)
}

function Add-U16([System.Collections.Generic.List[byte]] $Buffer, [UInt16] $Value) {
    $Buffer.AddRange([BitConverter]::GetBytes($Value))
}

function Add-U32([System.Collections.Generic.List[byte]] $Buffer, [UInt32] $Value) {
    $Buffer.AddRange([BitConverter]::GetBytes($Value))
}

function Add-U64([System.Collections.Generic.List[byte]] $Buffer, [UInt64] $Value) {
    $Buffer.AddRange([BitConverter]::GetBytes($Value))
}

function Add-I64([System.Collections.Generic.List[byte]] $Buffer, [Int64] $Value) {
    $Buffer.AddRange([BitConverter]::GetBytes($Value))
}

function New-Yuy2Frame([int] $FrameIndex) {
    $width = 1280
    $height = 720
    $payload = [byte[]]::new($width * $height * 2)
    $barWidth = [Math]::Max(1, [int]($width / 8))
    $offset = 0
    for ($y = 0; $y -lt $height; $y++) {
        for ($x = 0; $x -lt $width; $x += 2) {
            $bar = [int]((($x + ($FrameIndex * 17)) / $barWidth) % 8)
            $luma = [byte](32 + (($bar * 25 + ($y % 16)) % 200))
            $u = [byte](64 + (($bar * 19 + $FrameIndex) % 128))
            $v = [byte](64 + ((7 - $bar) * 19 + $FrameIndex) % 128)
            $payload[$offset] = $luma
            $payload[$offset + 1] = $u
            $payload[$offset + 2] = [byte]([Math]::Min(235, $luma + 8))
            $payload[$offset + 3] = $v
            $offset += 4
        }
    }
    Write-Output -NoEnumerate $payload
}

function New-FramePacket([UInt64] $Generation, [UInt64] $Sequence, [Int64] $Timestamp, [byte[]] $Payload) {
    $header = [System.Collections.Generic.List[byte]]::new(52)
    $header.AddRange([Text.Encoding]::ASCII.GetBytes('GPAKVC01'))
    Add-U16 $header 1
    Add-U16 $header 52
    Add-U64 $header $Generation
    Add-U64 $header $Sequence
    Add-I64 $header $Timestamp
    Add-U32 $header 1280
    Add-U32 $header 720
    Add-U32 $header ([UInt32]$Payload.Length)
    Add-U32 $header 0
    Write-Output -NoEnumerate ([byte[]]($header.ToArray() + $Payload))
}

function New-SessionToken {
    $bytes = [byte[]]::new(16)
    $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $generator.GetBytes($bytes) } finally { $generator.Dispose() }
    return (($bytes | ForEach-Object { $_.ToString('x2') }) -join '')
}

function Write-Evidence([string] $Path, [object] $Evidence) {
    if ([string]::IsNullOrWhiteSpace($Path)) { return }
    $path = Require-AbsolutePath $Path 'Output'
    $directory = Split-Path -Parent $path
    if ($directory) { New-Item -ItemType Directory -Force -Path $directory | Out-Null }
    $temporary = "${path}.${PID}.tmp"
    [IO.File]::WriteAllText(
        $temporary,
        (($Evidence | ConvertTo-Json -Depth 8) + [Environment]::NewLine),
        [Text.UTF8Encoding]::new($false)
    )
    Move-Item -LiteralPath $temporary -Destination $path -Force
}

$resolvedSidecar = Require-AbsolutePath $SidecarPath 'SidecarPath'
if (-not (Test-Path -LiteralPath $resolvedSidecar -PathType Leaf)) {
    throw "SidecarPath 不存在：$resolvedSidecar"
}

$token = New-SessionToken
$pipeName = "GpAutoLive-AkVirtualCamera-$token"
$startInfo = [Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $resolvedSidecar
$startInfo.Arguments = '--session-token-stdin'
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardInput = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true
$process = [Diagnostics.Process]::new()
$process.StartInfo = $startInfo
$null = $process.Start()
$stdoutTask = $process.StandardOutput.ReadToEndAsync()
$stderrTask = $process.StandardError.ReadToEndAsync()
$stdin = $process.StandardInput.BaseStream
$tokenBytes = [Text.Encoding]::ASCII.GetBytes($token + [Environment]::NewLine)
$stdin.Write($tokenBytes, 0, $tokenBytes.Length)
$stdin.Flush()
$stdin.Close()

$frames = 0
$lastSequence = [UInt64]0
$sequenceMonotonic = $true
$patterns = [System.Collections.Generic.List[byte[]]]::new()
for ($patternIndex = 0; $patternIndex -lt 8; $patternIndex++) {
    # Precompute a small ring of moving bars so the CPU test also proves that
    # downstream consumers receive changing frames, without regenerating a
    # 1.8 MiB payload inside the 30fps send loop.
    [void]$patterns.Add([byte[]](New-Yuy2Frame $patternIndex))
}
$connectError = $null
$exitCode = $null
$stopwatch = [Diagnostics.Stopwatch]::StartNew()
try {
    $client = [IO.Pipes.NamedPipeClientStream]::new('.', $pipeName, [IO.Pipes.PipeDirection]::Out, [IO.Pipes.PipeOptions]::None)
    try {
        $client.Connect(5000)
        $stream = [IO.BinaryWriter]::new($client)
        try {
            $minimumFrames = [Math]::Max(1, $Seconds * 30)
            for ($index = 1; $index -le $minimumFrames; $index++) {
                $timestamp = [int64]($index * 333333)
                $payload = $patterns[($index - 1) % $patterns.Count]
                $packet = New-FramePacket 1 ([UInt64]$index) $timestamp $payload
                $stream.Write($packet)
                $stream.Flush()
                if ($index -le $lastSequence) { $sequenceMonotonic = $false }
                $lastSequence = [UInt64]$index
                $frames++
                $targetMs = [int](($index * 1000) / 30)
                $remaining = $targetMs - $stopwatch.ElapsedMilliseconds
                if ($remaining -gt 0) { Start-Sleep -Milliseconds ([Math]::Min(50, $remaining)) }
            }
        } finally {
            $stream.Dispose()
        }
    } finally {
        $client.Dispose()
    }
    if (-not $process.WaitForExit(5000)) {
        $process.Kill()
        $process.WaitForExit(5000)
    }
} catch {
    $connectError = $_.Exception.Message
} finally {
    if (-not $process.HasExited) {
        $process.Kill()
        $process.WaitForExit(5000)
    }
    $exitCode = $process.ExitCode
    $stdoutText = $stdoutTask.GetAwaiter().GetResult()
    $stderrText = $stderrTask.GetAwaiter().GetResult()
    $process.Dispose()
}

$evidence = [ordered]@{
    schemaVersion = 1
    gate = 'akvirtualcamera-cpu-test-pattern'
    status = if ($frames -gt 0 -and $sequenceMonotonic -and $exitCode -eq 0) { 'passed' } else { 'blocked' }
    format = 'YUY2'
    width = 1280
    height = 720
    fps = 30
    frames = $frames
    sequenceMonotonic = $sequenceMonotonic
    sidecarExitCode = $exitCode
    stdout = $stdoutText.Trim()
    stderr = $stderrText.Trim()
    error = $connectError
}
Write-Evidence $Output $evidence
$evidence | ConvertTo-Json -Depth 8
if ($evidence.status -ne 'passed') { exit 2 }
