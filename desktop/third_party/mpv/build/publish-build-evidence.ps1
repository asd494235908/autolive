param(
    [Parameter(Mandatory = $true)]
    [string] $SourceRoot,
    [Parameter(Mandatory = $true)]
    [string] $MpvRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-ChildPath([string] $Root, [string] $Candidate, [string] $Label) {
    $separator = [System.IO.Path]::DirectorySeparatorChar
    $rootFull = [System.IO.Path]::GetFullPath($Root).TrimEnd($separator)
    $candidateFull = [System.IO.Path]::GetFullPath($Candidate)
    if (-not $candidateFull.StartsWith(($rootFull + $separator), [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label must stay strictly inside mpvRoot: $candidateFull"
    }
    return $candidateFull
}

function Remove-VerifiedTree([string] $Root, [string] $Target) {
    $verified = Assert-ChildPath $Root $Target 'cleanup target'
    if (Test-Path -LiteralPath $verified) {
        Remove-Item -Recurse -Force -LiteralPath $verified
    }
}

$mpvRootFull = (Resolve-Path -LiteralPath $MpvRoot).Path
$sourceRootFull = (Resolve-Path -LiteralPath $SourceRoot).Path
$sourceReport = Join-Path $sourceRootFull 'reproducible-build-report.json'
$sourceEvidence = Join-Path $sourceRootFull 'build-evidence'
if (-not (Test-Path -LiteralPath $sourceReport -PathType Leaf) -or -not (Test-Path -LiteralPath $sourceEvidence -PathType Container)) {
    throw 'publish source is missing report or build-evidence'
}
$report = Get-Content -Raw -LiteralPath $sourceReport | ConvertFrom-Json
$sourceEvidenceFull = [System.IO.Path]::GetFullPath($sourceEvidence).TrimEnd([System.IO.Path]::DirectorySeparatorChar)
$declared = @($report.artifacts | ForEach-Object { [string]$_.path } | Sort-Object)
$actual = @(Get-ChildItem -Recurse -File -LiteralPath $sourceEvidence | ForEach-Object {
    $_.FullName.Substring($sourceEvidenceFull.Length + 1).Replace([char]92, [char]47)
} | Sort-Object)
if (Compare-Object -ReferenceObject $declared -DifferenceObject $actual) {
    throw 'publish source evidence set does not match report'
}
foreach ($artifact in $report.artifacts) {
    $source = Join-Path $sourceEvidence ([string]$artifact.path)
    $item = Get-Item -LiteralPath $source
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "publish source must not contain reparse points: $source"
    }
    $digest = (Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash.ToLowerInvariant()
    if ($item.Length -ne [long]$artifact.size -or $digest -ne [string]$artifact.sha256) {
        throw "publish source hash mismatch: $($artifact.path)"
    }
}

$transactionId = [guid]::NewGuid().ToString('N')
$stageRoot = Assert-ChildPath $mpvRootFull (Join-Path $mpvRootFull ".phase7a-publish-$transactionId") 'staging'
$backupRoot = Assert-ChildPath $mpvRootFull (Join-Path $mpvRootFull ".phase7a-backup-$transactionId") 'backup'
$stageEvidence = Join-Path $stageRoot 'build-evidence'
$stageReport = Join-Path $stageRoot 'reproducible-build-report.json'
$targetEvidence = Join-Path $mpvRootFull 'build-evidence'
$targetReport = Join-Path $mpvRootFull 'reproducible-build-report.json'

try {
    New-Item -ItemType Directory -Path $stageEvidence | Out-Null
    foreach ($artifact in $report.artifacts) {
        $relative = [string]$artifact.path
        $source = Join-Path $sourceEvidence $relative
        $destination = Join-Path $stageEvidence $relative
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
        Copy-Item -LiteralPath $source -Destination $destination
    }
    Copy-Item -LiteralPath $sourceReport -Destination $stageReport
    $stageEvidenceFull = [System.IO.Path]::GetFullPath($stageEvidence).TrimEnd([System.IO.Path]::DirectorySeparatorChar)
    $staged = @(Get-ChildItem -Recurse -File -LiteralPath $stageEvidence | ForEach-Object {
        $_.FullName.Substring($stageEvidenceFull.Length + 1).Replace([char]92, [char]47)
    } | Sort-Object)
    if (Compare-Object -ReferenceObject $declared -DifferenceObject $staged) {
        throw 'staging evidence set does not match report'
    }
    foreach ($artifact in $report.artifacts) {
        $path = Join-Path $stageEvidence ([string]$artifact.path)
        if ((Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant() -ne [string]$artifact.sha256) {
            throw "staging hash mismatch: $($artifact.path)"
        }
    }

    New-Item -ItemType Directory -Path $backupRoot | Out-Null
    $oldEvidenceMoved = $false
    $oldReportMoved = $false
    $newEvidenceMoved = $false
    $newReportMoved = $false
    try {
        if (Test-Path -LiteralPath $targetEvidence) {
            Move-Item -LiteralPath $targetEvidence -Destination (Join-Path $backupRoot 'build-evidence')
            $oldEvidenceMoved = $true
        }
        if (Test-Path -LiteralPath $targetReport) {
            Move-Item -LiteralPath $targetReport -Destination (Join-Path $backupRoot 'reproducible-build-report.json')
            $oldReportMoved = $true
        }
        Move-Item -LiteralPath $stageEvidence -Destination $targetEvidence
        $newEvidenceMoved = $true
        Move-Item -LiteralPath $stageReport -Destination $targetReport
        $newReportMoved = $true
    } catch {
        if ($newReportMoved -and (Test-Path -LiteralPath $targetReport)) { Remove-Item -Force -LiteralPath $targetReport }
        if ($newEvidenceMoved -and (Test-Path -LiteralPath $targetEvidence)) { Remove-VerifiedTree $mpvRootFull $targetEvidence }
        if ($oldReportMoved) { Move-Item -LiteralPath (Join-Path $backupRoot 'reproducible-build-report.json') -Destination $targetReport }
        if ($oldEvidenceMoved) { Move-Item -LiteralPath (Join-Path $backupRoot 'build-evidence') -Destination $targetEvidence }
        throw
    }
    Remove-VerifiedTree $mpvRootFull $backupRoot
} finally {
    Remove-VerifiedTree $mpvRootFull $stageRoot
}
