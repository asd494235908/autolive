[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InputPath,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath,

    [ValidateSet('NoPlayback30m', 'JointPlayback30m', 'IdleCpu')]
    [string]$Scenario = 'NoPlayback30m'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($PSVersionTable.PSVersion.Major -lt 7) {
    throw 'This baseline evaluator requires PowerShell 7 (pwsh).'
}

# Keep the parser and report bounded even when the input is user-supplied JSON.
$maxInputBytes = 16MB
$maxOutputBytes = 64KB
$maxSamples = 36001
$maxStringLength = 256
$maxMetricBytes = 1L -shl 50
$maxObjectCount = 1000000
$memoryMiB = 1MB

function Fail([string]$Code) {
    throw [InvalidOperationException]::new($Code)
}

function Resolve-RegularFile([string]$Path, [string]$Name) {
    if ([string]::IsNullOrWhiteSpace($Path)) {
        Fail "${Name}_missing"
    }

    try {
        $resolved = [IO.Path]::GetFullPath($Path)
    }
    catch {
        Fail "${Name}_invalid"
    }

    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        Fail "${Name}_missing"
    }

    try {
        $item = Get-Item -LiteralPath $resolved -ErrorAction Stop
    }
    catch {
        Fail "${Name}_unreadable"
    }

    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        Fail "${Name}_reparse_point"
    }

    if ($item.Length -le 0 -or $item.Length -gt $maxInputBytes) {
        Fail "${Name}_size"
    }

    return $item
}

function Get-PropertyNames([object]$Object, [string]$Scope) {
    if ($null -eq $Object -or $Object -is [Collections.IEnumerable] -and $Object -isnot [string]) {
        Fail "${Scope}_object_required"
    }

    return @($Object.PSObject.Properties | ForEach-Object { [string]$_.Name })
}

function Assert-ExactProperties([object]$Object, [string[]]$Expected, [string]$Scope) {
    $actual = @(Get-PropertyNames $Object $Scope)
    if ($actual.Count -ne $Expected.Count) {
        Fail "${Scope}_fields"
    }

    foreach ($name in $actual) {
        if ($Expected -notcontains $name) {
            Fail "${Scope}_unknown_field"
        }
    }
}

function Get-RequiredProperty([object]$Object, [string]$Name, [string]$Scope) {
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property) {
        Fail "${Scope}_${Name}_missing"
    }

    return $property.Value
}

function Assert-String([object]$Value, [string]$Code, [int]$MaxLength = $maxStringLength) {
    if ($Value -isnot [string] -or [string]::IsNullOrWhiteSpace([string]$Value) -or $Value.Length -gt $MaxLength) {
        Fail $Code
    }

    if ($Value -match '[\x00-\x1F]') {
        Fail "${Code}_control"
    }
}

function Assert-Integer([object]$Value, [string]$Code, [long]$Minimum, [long]$Maximum) {
    if ($null -eq $Value -or $Value -is [bool]) {
        Fail $Code
    }

    try {
        $number = [decimal]$Value
    }
    catch {
        Fail $Code
    }

    if ($number -ne [decimal]::Truncate($number) -or $number -lt $Minimum -or $number -gt $Maximum) {
        Fail $Code
    }

    return [long]$number
}

function Assert-NullableInteger([object]$Value, [string]$Code, [long]$Minimum, [long]$Maximum) {
    if ($null -eq $Value) {
        return $null
    }

    return Assert-Integer $Value $Code $Minimum $Maximum
}

function Assert-Number([object]$Value, [string]$Code, [double]$Minimum, [double]$Maximum) {
    if ($null -eq $Value -or $Value -is [bool]) {
        Fail $Code
    }

    try {
        $number = [double]$Value
    }
    catch {
        Fail $Code
    }

    if ([double]::IsNaN($number) -or [double]::IsInfinity($number) -or $number -lt $Minimum -or $number -gt $Maximum) {
        Fail $Code
    }

    return $number
}

function Assert-NullableNumber([object]$Value, [string]$Code, [double]$Minimum, [double]$Maximum) {
    if ($null -eq $Value) {
        return $null
    }

    return Assert-Number $Value $Code $Minimum $Maximum
}

function Assert-UtcTimestamp([object]$Value, [string]$Code) {
    Assert-String $Value $Code 64
    if ($Value -notmatch '(Z|[+-][0-9]{2}:[0-9]{2})$') {
        Fail "${Code}_timezone"
    }

    try {
        [DateTimeOffset]::Parse(
            $Value,
            [Globalization.CultureInfo]::InvariantCulture,
            [Globalization.DateTimeStyles]::RoundtripKind) | Out-Null
    }
    catch {
        Fail "${Code}_format"
    }
}

function Assert-NoDuplicateJsonProperties([System.Text.Json.JsonElement]$Element) {
    if ($Element.ValueKind -eq [System.Text.Json.JsonValueKind]::Object) {
        $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($property in $Element.EnumerateObject()) {
            if (-not $names.Add($property.Name)) {
                Fail 'input_duplicate_field'
            }

            Assert-NoDuplicateJsonProperties $property.Value
        }
    }
    elseif ($Element.ValueKind -eq [System.Text.Json.JsonValueKind]::Array) {
        foreach ($item in $Element.EnumerateArray()) {
            Assert-NoDuplicateJsonProperties $item
        }
    }
}

function Assert-EqualNumber([object]$Actual, [double]$Expected, [string]$Code) {
    if ($null -eq $Actual) {
        Fail $Code
    }

    $number = Assert-Number $Actual $Code (-[double]::MaxValue) ([double]::MaxValue)
    if ($number -ne $Expected) {
        Fail $Code
    }
}

function Assert-NullableEqualNumber([object]$Actual, [object]$Expected, [string]$Code) {
    if ($null -eq $Expected) {
        if ($null -ne $Actual) {
            Fail $Code
        }

        return
    }

    Assert-EqualNumber $Actual $Expected $Code
}

function Read-BaselineReport([string]$Path) {
    $file = Resolve-RegularFile $Path 'input'
    $raw = $null
    try {
        $raw = [IO.File]::ReadAllText($file.FullName)
        $document = [System.Text.Json.JsonDocument]::Parse($raw)
        try {
            if ($document.RootElement.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) {
                Fail 'input_root_object_required'
            }

            $samplesIsArray = $false
            foreach ($property in $document.RootElement.EnumerateObject()) {
                if ($property.Name -ceq 'samples') {
                    $samplesIsArray = $property.Value.ValueKind -eq [System.Text.Json.JsonValueKind]::Array
                    break
                }
            }
            if (-not $samplesIsArray) {
                Fail 'samples_array_required'
            }

            Assert-NoDuplicateJsonProperties $document.RootElement
        }
        finally {
            $document.Dispose()
        }

        # Keep timestamps as strings so their wire shape is validated explicitly.
        $report = $raw | ConvertFrom-Json -Depth 8 -DateKind String
    }
    catch {
        Fail 'input_invalid_json'
    }

    if ($null -eq $report -or $report -is [Collections.IEnumerable] -and $report -isnot [string]) {
        Fail 'input_root_object_required'
    }

    $rootFields = @(
        'schema_version', 'captured_at_utc', 'platform', 'executable',
        'duration_seconds', 'interval_milliseconds', 'warmup_milliseconds',
        'logical_processor_count', 'status', 'summary', 'samples')
    Assert-ExactProperties $report $rootFields 'root'

    $schema = Assert-Integer (Get-RequiredProperty $report 'schema_version' 'root') 'schema_version' 1 1
    $null = $schema
    Assert-UtcTimestamp (Get-RequiredProperty $report 'captured_at_utc' 'root') 'captured_at_utc'

    $platform = Get-RequiredProperty $report 'platform' 'root'
    Assert-String $platform 'platform' 32
    if ($platform -cne 'windows') {
        Fail 'platform_unsupported'
    }

    $executable = Get-RequiredProperty $report 'executable' 'root'
    Assert-String $executable 'executable' 128
    if ([IO.Path]::GetFileName($executable) -cne $executable -or $executable -match '[\\/]') {
        Fail 'executable_must_be_name_only'
    }

    $duration = Assert-Integer (Get-RequiredProperty $report 'duration_seconds' 'root') 'duration_seconds' 1 3600
    $interval = Assert-Integer (Get-RequiredProperty $report 'interval_milliseconds' 'root') 'interval_milliseconds' 100 60000
    $warmup = Assert-Integer (Get-RequiredProperty $report 'warmup_milliseconds' 'root') 'warmup_milliseconds' 0 60000
    $logicalProcessors = Assert-Integer (Get-RequiredProperty $report 'logical_processor_count' 'root') 'logical_processor_count' 1 512

    $status = Get-RequiredProperty $report 'status' 'root'
    Assert-String $status 'status' 32
    if (@('not_started', 'running', 'exited', 'unavailable', 'completed') -notcontains $status) {
        Fail 'status_unsupported'
    }

    $summary = Get-RequiredProperty $report 'summary' 'root'
    $summaryFields = @(
        'sample_count', 'private_working_set_min_bytes', 'private_working_set_max_bytes',
        'working_set_min_bytes', 'working_set_max_bytes', 'gdi_object_count_min',
        'gdi_object_count_max', 'user_object_count_min', 'user_object_count_max',
        'cpu_percent_max')
    Assert-ExactProperties $summary $summaryFields 'summary'

    $samplesValue = Get-RequiredProperty $report 'samples' 'root'
    if ($null -eq $samplesValue -or $samplesValue -is [string]) {
        Fail 'samples_array_required'
    }

    $samples = @($samplesValue)
    if ($samples.Count -gt $maxSamples) {
        Fail 'samples_limit'
    }

    $sampleFields = @(
        'captured_at_utc', 'elapsed_ms', 'private_working_set_bytes',
        'working_set_bytes', 'thread_count', 'handle_count',
        'gdi_object_count', 'user_object_count', 'cpu_percent')
    $parsedSamples = [Collections.Generic.List[object]]::new()
    $previousElapsed = $null
    foreach ($sample in $samples) {
        Assert-ExactProperties $sample $sampleFields 'sample'
        Assert-UtcTimestamp (Get-RequiredProperty $sample 'captured_at_utc' 'sample') 'sample_captured_at_utc'
        $elapsed = Assert-Integer (Get-RequiredProperty $sample 'elapsed_ms' 'sample') 'sample_elapsed_ms' 0 3601000
        if ($null -ne $previousElapsed -and $elapsed -le $previousElapsed) {
            Fail 'sample_elapsed_not_monotonic'
        }

        $previousElapsed = $elapsed
        $private = Assert-Integer (Get-RequiredProperty $sample 'private_working_set_bytes' 'sample') 'sample_private_working_set_bytes' 0 $maxMetricBytes
        $working = Assert-Integer (Get-RequiredProperty $sample 'working_set_bytes' 'sample') 'sample_working_set_bytes' 0 $maxMetricBytes
        $threads = Assert-Integer (Get-RequiredProperty $sample 'thread_count' 'sample') 'sample_thread_count' 0 $maxObjectCount
        $handles = Assert-Integer (Get-RequiredProperty $sample 'handle_count' 'sample') 'sample_handle_count' 0 $maxObjectCount
        $gdi = Assert-NullableInteger (Get-RequiredProperty $sample 'gdi_object_count' 'sample') 'sample_gdi_object_count' 0 $maxObjectCount
        $user = Assert-NullableInteger (Get-RequiredProperty $sample 'user_object_count' 'sample') 'sample_user_object_count' 0 $maxObjectCount
        $cpu = Assert-NullableNumber (Get-RequiredProperty $sample 'cpu_percent' 'sample') 'sample_cpu_percent' 0 100
        $parsedSamples.Add([pscustomobject]@{
                elapsed_ms = $elapsed
                private_working_set_bytes = $private
                working_set_bytes = $working
                thread_count = $threads
                handle_count = $handles
                gdi_object_count = $gdi
                user_object_count = $user
                cpu_percent = $cpu
            })
    }

    $summaryCount = Assert-Integer (Get-RequiredProperty $summary 'sample_count' 'summary') 'summary_sample_count' 0 $maxSamples
    if ($summaryCount -ne $samples.Count) {
        Fail 'summary_sample_count_mismatch'
    }

    $allPrivate = @($parsedSamples | ForEach-Object { $_.private_working_set_bytes })
    $allWorking = @($parsedSamples | ForEach-Object { $_.working_set_bytes })
    $allGdi = @($parsedSamples | Where-Object { $null -ne $_.gdi_object_count } | ForEach-Object { $_.gdi_object_count })
    $allUser = @($parsedSamples | Where-Object { $null -ne $_.user_object_count } | ForEach-Object { $_.user_object_count })
    $allCpu = @($parsedSamples | Where-Object { $null -ne $_.cpu_percent } | ForEach-Object { $_.cpu_percent })

    function Get-MinimumOrNull([object[]]$Values) {
        if ($Values.Count -eq 0) { return $null }
        return [double]($Values | Measure-Object -Minimum).Minimum
    }

    function Get-MaximumOrNull([object[]]$Values) {
        if ($Values.Count -eq 0) { return $null }
        return [double]($Values | Measure-Object -Maximum).Maximum
    }

    Assert-NullableEqualNumber (Get-RequiredProperty $summary 'private_working_set_min_bytes' 'summary') (Get-MinimumOrNull $allPrivate) 'summary_private_min_mismatch'
    Assert-NullableEqualNumber (Get-RequiredProperty $summary 'private_working_set_max_bytes' 'summary') (Get-MaximumOrNull $allPrivate) 'summary_private_max_mismatch'
    Assert-NullableEqualNumber (Get-RequiredProperty $summary 'working_set_min_bytes' 'summary') (Get-MinimumOrNull $allWorking) 'summary_working_min_mismatch'
    Assert-NullableEqualNumber (Get-RequiredProperty $summary 'working_set_max_bytes' 'summary') (Get-MaximumOrNull $allWorking) 'summary_working_max_mismatch'

    $gdiMin = Get-MinimumOrNull $allGdi
    $gdiMax = Get-MaximumOrNull $allGdi
    $userMin = Get-MinimumOrNull $allUser
    $userMax = Get-MaximumOrNull $allUser
    $cpuMax = if ($allCpu.Count -gt 0) { [double][Math]::Round((Get-MaximumOrNull $allCpu), 2) } else { $null }
    Assert-NullableEqualNumber (Get-RequiredProperty $summary 'gdi_object_count_min' 'summary') $gdiMin 'summary_gdi_min_mismatch'
    Assert-NullableEqualNumber (Get-RequiredProperty $summary 'gdi_object_count_max' 'summary') $gdiMax 'summary_gdi_max_mismatch'
    Assert-NullableEqualNumber (Get-RequiredProperty $summary 'user_object_count_min' 'summary') $userMin 'summary_user_min_mismatch'
    Assert-NullableEqualNumber (Get-RequiredProperty $summary 'user_object_count_max' 'summary') $userMax 'summary_user_max_mismatch'
    Assert-NullableEqualNumber (Get-RequiredProperty $summary 'cpu_percent_max' 'summary') $cpuMax 'summary_cpu_max_mismatch'

    if ($samples.Count -gt 0 -and $previousElapsed -gt ($duration * 1000 + $interval)) {
        Fail 'sample_elapsed_exceeds_requested_duration'
    }

    return [pscustomobject]@{
        executable = $executable
        duration_seconds = $duration
        interval_milliseconds = $interval
        warmup_milliseconds = $warmup
        logical_processor_count = $logicalProcessors
        status = $status
        samples = $parsedSamples.ToArray()
        sample_count = $samples.Count
        observed_duration_seconds = if ($samples.Count -ge 2) {
            [Math]::Round(($parsedSamples[$samples.Count - 1].elapsed_ms - $parsedSamples[0].elapsed_ms) / 1000d, 3)
        }
        else { 0d }
        private_min_bytes = if ($allPrivate.Count -gt 0) { [int64](Get-MinimumOrNull $allPrivate) } else { $null }
        private_max_bytes = if ($allPrivate.Count -gt 0) { [int64](Get-MaximumOrNull $allPrivate) } else { $null }
        private_first_bytes = if ($samples.Count -gt 0) { [int64]$parsedSamples[0].private_working_set_bytes } else { $null }
        private_last_bytes = if ($samples.Count -gt 0) { [int64]$parsedSamples[$samples.Count - 1].private_working_set_bytes } else { $null }
        cpu_values = $allCpu
    }
}

function New-Check([string]$Name, [string]$Status, [object]$Actual, [object]$Limit, [string]$Unit, [string]$Message) {
    [ordered]@{
        name = $Name
        status = $Status
        actual = $Actual
        limit = $Limit
        unit = $Unit
        message = $Message
    }
}

function Get-P95([double[]]$Values) {
    if ($Values.Count -eq 0) { return $null }
    $sorted = @($Values | Sort-Object)
    $rank = [Math]::Max(1, [int][Math]::Ceiling($sorted.Count * 0.95))
    return [double]$sorted[$rank - 1]
}

function Write-BoundedReport([object]$Report, [string]$Path) {
    try {
        $output = [IO.Path]::GetFullPath($Path)
    }
    catch {
        Fail 'output_invalid'
    }

    $parent = Split-Path -Parent $output
    if ([string]::IsNullOrWhiteSpace($parent)) {
        Fail 'output_directory_missing'
    }

    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }

    $parentItem = Get-Item -LiteralPath $parent -ErrorAction Stop
    if (($parentItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        Fail 'output_directory_reparse_point'
    }

    if (Test-Path -LiteralPath $output -PathType Leaf) {
        $existing = Get-Item -LiteralPath $output
        if (($existing.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            Fail 'output_reparse_point'
        }
    }

    $json = $Report | ConvertTo-Json -Depth 8 -Compress
    $bytes = [Text.Encoding]::UTF8.GetByteCount($json)
    if ($bytes -gt $maxOutputBytes) {
        Fail 'output_size'
    }

    $temporary = Join-Path $parent ('.baseline-evaluation-' + [Guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [IO.File]::WriteAllText($temporary, $json, [Text.UTF8Encoding]::new($false))
        [IO.File]::Move($temporary, $output, $true)
    }
    finally {
        if (Test-Path -LiteralPath $temporary -PathType Leaf) {
            Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
        }
    }
}

$parsed = Read-BaselineReport $InputPath
$checks = [Collections.Generic.List[object]]::new()

$statusCheck = if ($parsed.status -eq 'completed') {
    New-Check 'completed_status' 'passed' $parsed.status 'completed' 'state' '采样器正常完成。'
}
else {
    New-Check 'completed_status' 'not_ready' $parsed.status 'completed' 'state' '采样未以 completed 结束，不能作为性能门禁证据。'
}
$checks.Add($statusCheck)

if ($Scenario -eq 'IdleCpu') {
    $minimumSamples = 3
    $minimumDuration = 30d
    $durationCheck = if ($parsed.sample_count -ge $minimumSamples -and $parsed.observed_duration_seconds -ge $minimumDuration) {
        New-Check 'observation_window' 'passed' $parsed.observed_duration_seconds $minimumDuration 'seconds' '已达到 30 秒且包含多个样本。'
    }
    else {
        New-Check 'observation_window' 'not_ready' $parsed.observed_duration_seconds $minimumDuration 'seconds' '观测窗口不足；单次采样不能证明 CPU P95。'
    }
    $checks.Add($durationCheck)

    $cpuP95 = Get-P95 ([double[]]$parsed.cpu_values)
    $cpuCheck = if ($null -eq $cpuP95 -or $parsed.cpu_values.Count -lt $minimumSamples) {
        New-Check 'idle_cpu_p95' 'not_ready' $cpuP95 1.0 'percent' '缺少至少三个有效 CPU 样本。'
    }
    elseif ($cpuP95 -lt 1.0) {
        New-Check 'idle_cpu_p95' 'passed' ([Math]::Round($cpuP95, 3)) 1.0 'percent' 'CPU P95 小于 1%。'
    }
    else {
        New-Check 'idle_cpu_p95' 'failed' ([Math]::Round($cpuP95, 3)) 1.0 'percent' 'CPU P95 未达到小于 1% 的目标。'
    }
    $checks.Add($cpuCheck)
}
else {
    $minimumDuration = 1800d
    $minimumSamples = 30
    $durationCheck = if ($parsed.sample_count -ge $minimumSamples -and $parsed.observed_duration_seconds -ge $minimumDuration) {
        New-Check 'observation_window' 'passed' $parsed.observed_duration_seconds $minimumDuration 'seconds' '已达到 30 分钟真实样本跨度。'
    }
    else {
        New-Check 'observation_window' 'not_ready' $parsed.observed_duration_seconds $minimumDuration 'seconds' '需要至少 30 分钟真实样本跨度和 30 个样本；不能使用 duration_seconds 字段冒充。'
    }
    $checks.Add($durationCheck)

    $growthLimit = if ($Scenario -eq 'NoPlayback30m') { 20 * $memoryMiB } else { 40 * $memoryMiB }
    $peakDelta = if ($null -ne $parsed.private_first_bytes) { $parsed.private_max_bytes - $parsed.private_first_bytes } else { $null }
    $lastDelta = if ($null -ne $parsed.private_first_bytes) { $parsed.private_last_bytes - $parsed.private_first_bytes } else { $null }
    $peakCheck = if ($null -eq $peakDelta -or $parsed.sample_count -lt 2) {
        New-Check 'private_working_set_peak_delta' 'not_ready' $peakDelta $growthLimit 'bytes' '至少需要两个有效样本。'
    }
    elseif ($peakDelta -le $growthLimit) {
        New-Check 'private_working_set_peak_delta' 'passed' $peakDelta $growthLimit 'bytes' '峰值相对首个样本未超过内存增长预算。'
    }
    else {
        New-Check 'private_working_set_peak_delta' 'failed' $peakDelta $growthLimit 'bytes' '峰值相对首个样本超过内存增长预算。'
    }
    $checks.Add($peakCheck)

    $lastCheck = if ($null -eq $lastDelta -or $parsed.sample_count -lt 2) {
        New-Check 'private_working_set_last_delta' 'not_ready' $lastDelta $growthLimit 'bytes' '至少需要两个有效样本。'
    }
    elseif ($lastDelta -le $growthLimit) {
        New-Check 'private_working_set_last_delta' 'passed' $lastDelta $growthLimit 'bytes' '末样本相对首个样本未超过内存增长预算。'
    }
    else {
        New-Check 'private_working_set_last_delta' 'failed' $lastDelta $growthLimit 'bytes' '末样本相对首个样本超过内存增长预算。'
    }
    $checks.Add($lastCheck)

    # The external Process API cannot prove the target's post-GC stability.
    $checks.Add((New-Check 'gc_stability' 'not_ready' $null 'application_evidence_required' 'state' '采集器不伪造跨进程 GC 证据；需结合应用内 GC 快照和长稳报告确认。'))
}

$checkArray = @($checks)
$hasFailed = @($checkArray | Where-Object { $_.status -eq 'failed' }).Count -gt 0
$hasNotReady = @($checkArray | Where-Object { $_.status -eq 'not_ready' }).Count -gt 0
$overallStatus = if ($hasFailed) { 'failed' } elseif ($hasNotReady) { 'not_ready' } else { 'passed' }
$exitCode = if ($overallStatus -eq 'passed') { 0 } elseif ($overallStatus -eq 'not_ready') { 2 } else { 3 }

$evaluation = [ordered]@{
    schema_version = 1
    report_type = 'process_baseline_evaluation'
    evaluated_at_utc = [DateTimeOffset]::UtcNow.ToString('O')
    platform = 'windows'
    scenario = switch ($Scenario) {
        'NoPlayback30m' { 'no_playback_30m' }
        'JointPlayback30m' { 'joint_playback_30m' }
        'IdleCpu' { 'idle_cpu' }
    }
    executable = $parsed.executable
    input_schema_version = 1
    input_status = $parsed.status
    sample_count = $parsed.sample_count
    observed_duration_seconds = $parsed.observed_duration_seconds
    cpu_sample_count = $parsed.cpu_values.Count
    cpu_p95_percent = if ($parsed.cpu_values.Count -gt 0) { [Math]::Round((Get-P95 ([double[]]$parsed.cpu_values)), 3) } else { $null }
    private_working_set_first_bytes = $parsed.private_first_bytes
    private_working_set_last_bytes = $parsed.private_last_bytes
    private_working_set_peak_bytes = $parsed.private_max_bytes
    status = $overallStatus
    checks = $checkArray
}

Write-BoundedReport $evaluation $OutputPath
Write-Output ("Baseline evaluation written: {0} (status={1})" -f [IO.Path]::GetFullPath($OutputPath), $overallStatus)
exit $exitCode
