[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = [IO.Path]::GetFullPath($PSScriptRoot)
$tool = Join-Path $scriptRoot 'evaluate-process-baseline.ps1'
$fixtureRoot = Join-Path (Split-Path -Parent $scriptRoot) 'tests/fixtures/performance'
$runner = (Get-Command pwsh -ErrorAction Stop).Source

function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw $Message
    }
}

function Invoke-Evaluation([string]$InputPath, [string]$Scenario, [string]$OutputPath) {
    $arguments = @(
        '-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
        '-File', $tool, '-InputPath', $InputPath, '-OutputPath', $OutputPath)
    if ($Scenario -ne 'NoPlayback30m') {
        $arguments += @('-Scenario', $Scenario)
    }

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $runner
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    foreach ($argument in $arguments) {
        $startInfo.ArgumentList.Add($argument)
    }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        Assert-Condition $process.Start() 'evaluation process did not start'
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        Assert-Condition $process.WaitForExit(10000) 'evaluation process exceeded 10 seconds'
        $process.WaitForExit()
        $stdoutText = $stdout.GetAwaiter().GetResult()
        $stderrText = $stderr.GetAwaiter().GetResult()
        $evaluation = $null
        if (Test-Path -LiteralPath $OutputPath -PathType Leaf) {
            $evaluation = Get-Content -LiteralPath $OutputPath -Raw | ConvertFrom-Json -Depth 8 -DateKind String
        }

        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            Evaluation = $evaluation
            Stdout = $stdoutText
            Stderr = $stderrText
        }
    }
    finally {
        $process.Dispose()
    }
}

function Get-Check([object]$Evaluation, [string]$Name) {
    return @($Evaluation.checks | Where-Object { $_.name -eq $Name })[0]
}

$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ('gpautolive-c6-eval-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempRoot | Out-Null

try {
    $singleOutput = Join-Path $tempRoot 'single.json'
    $single = Invoke-Evaluation (Join-Path $fixtureRoot 'baseline-single-sample-claims-30m.json') 'NoPlayback30m' $singleOutput
    Assert-Condition ($single.ExitCode -eq 2) 'single sample must return not_ready exit code 2'
    Assert-Condition ($null -ne $single.Evaluation) 'single sample must produce an evaluation report'
    Assert-Condition ($single.Evaluation.status -eq 'not_ready') 'single sample must not pass a 30-minute gate'
    Assert-Condition ((Get-Check $single.Evaluation 'observation_window').status -eq 'not_ready') 'single sample observation must be not_ready'

    $threeOutput = Join-Path $tempRoot 'three.json'
    $three = Invoke-Evaluation (Join-Path $fixtureRoot 'baseline-three-samples-30m.json') 'NoPlayback30m' $threeOutput
    Assert-Condition ($three.ExitCode -eq 2) 'three samples must remain not_ready because the trend evidence is sparse and GC evidence is absent'
    Assert-Condition ((Get-Check $three.Evaluation 'gc_stability').status -eq 'not_ready') 'external process sampling must not invent GC stability evidence'

    $idleOutput = Join-Path $tempRoot 'idle.json'
    $idle = Invoke-Evaluation (Join-Path $fixtureRoot 'baseline-idle-cpu-over-budget.json') 'IdleCpu' $idleOutput
    Assert-Condition ($idle.ExitCode -eq 3) 'CPU over-budget fixture must return failed exit code 3'
    Assert-Condition ($idle.Evaluation.status -eq 'failed') 'CPU over-budget fixture must fail'
    Assert-Condition ((Get-Check $idle.Evaluation 'idle_cpu_p95').actual -eq 1.5) 'CPU P95 must use nearest-rank P95 over valid samples'

    $unknownOutput = Join-Path $tempRoot 'unknown.json'
    $unknown = Invoke-Evaluation (Join-Path $fixtureRoot 'baseline-unknown-field.json') 'NoPlayback30m' $unknownOutput
    Assert-Condition ($unknown.ExitCode -eq 1) 'unknown input fields must be rejected'
    Assert-Condition ($null -eq $unknown.Evaluation) 'rejected input must not produce a report'

    $duplicateOutput = Join-Path $tempRoot 'duplicate.json'
    $duplicate = Invoke-Evaluation (Join-Path $fixtureRoot 'baseline-duplicate-field.json') 'NoPlayback30m' $duplicateOutput
    Assert-Condition ($duplicate.ExitCode -eq 1) 'duplicate input fields must be rejected'
    Assert-Condition ($null -eq $duplicate.Evaluation) 'duplicate input fields must not produce a report'

    Write-Output 'C6 baseline evaluation tests passed: single-sample guard, sparse-trend guard, CPU P95 threshold, strict/duplicate fields.'
}
finally {
    if (Test-Path -LiteralPath $tempRoot -PathType Container) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
