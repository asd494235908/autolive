param(
    [Parameter(Mandatory = $true)]
    [string] $OutputRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$recipeRoot = (Resolve-Path -LiteralPath $PSScriptRoot).Path
$mpvRoot = (Resolve-Path -LiteralPath (Join-Path $recipeRoot '..')).Path
$workspaceRoot = (Resolve-Path -LiteralPath (Join-Path $recipeRoot '..\..\..\..')).Path
$inputRoot = (Resolve-Path -LiteralPath (Join-Path $mpvRoot 'build-inputs')).Path
$lockPath = Join-Path $mpvRoot 'reproducible-build-lock.json'
$evidenceRoot = Join-Path $OutputRoot 'build-evidence'
$lock = Get-Content -Raw -LiteralPath $lockPath | ConvertFrom-Json
$lockSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $lockPath).Hash.ToLowerInvariant()
$tag = "autolive-mpv-phase7a:$($lockSha256.Substring(0, 16))"
$runId = [Guid]::NewGuid().ToString('N').ToLowerInvariant()
$containerName = "autolive-phase7a-$($lockSha256.Substring(0, 12))-$($runId.Substring(0, 12))"
$workVolumeName = "autolive-phase7a-work-$runId"

if (Test-Path -LiteralPath $OutputRoot) {
    if ((Get-ChildItem -Force -LiteralPath $OutputRoot | Select-Object -First 1)) {
        throw "要求全新空目录：$OutputRoot"
    }
} else {
    New-Item -ItemType Directory -Path $OutputRoot | Out-Null
}

Push-Location $workspaceRoot
try {
    & node 'desktop/tools/verify-mpv-reproducible-build-lock.mjs' '--require-metadata-complete'
    if ($LASTEXITCODE -ne 0) { throw 'Phase 7A 输入锁门禁失败' }
    & node 'desktop/tools/verify-mpv-reproducible-build-inputs.mjs' '--require-verified'
    if ($LASTEXITCODE -ne 0) { throw 'Phase 7A 输入字节门禁失败' }

    & docker build --network none --pull=false --tag $tag --file (Join-Path $recipeRoot 'Dockerfile') $mpvRoot
    if ($LASTEXITCODE -ne 0) { throw 'Phase 7A 配方镜像构建失败' }

    & docker volume inspect $workVolumeName *> $null
    if ($LASTEXITCODE -eq 0) { throw "Phase 7A 随机 work volume 已存在，拒绝复用：$workVolumeName" }
    $createdVolumeName = (& docker volume create `
        --driver local `
        --label "com.autolive.phase7a.run-id=$runId" `
        --label "com.autolive.phase7a.lock-sha256=$lockSha256" `
        --label 'com.autolive.phase7a.purpose=cold-build-work' `
        $workVolumeName).Trim()
    if ($LASTEXITCODE -ne 0 -or $createdVolumeName -ne $workVolumeName) {
        throw 'Phase 7A 唯一 work volume 创建失败'
    }
    $workVolume = (& docker volume inspect $workVolumeName | ConvertFrom-Json)[0]
    if (
        $workVolume.Name -ne $workVolumeName -or
        $workVolume.Driver -ne 'local' -or
        $workVolume.Labels.'com.autolive.phase7a.run-id' -ne $runId -or
        $workVolume.Labels.'com.autolive.phase7a.lock-sha256' -ne $lockSha256 -or
        $workVolume.Labels.'com.autolive.phase7a.purpose' -ne 'cold-build-work'
    ) {
        throw 'Phase 7A work volume 身份校验失败'
    }

    $containerId = (& docker create `
        --name $containerName `
        --network none `
        --read-only `
        --user '0:0' `
        --memory '4g' `
        --memory-swap '4g' `
        --mount "type=bind,src=$inputRoot,dst=/build-inputs,readonly" `
        --mount "type=bind,src=$OutputRoot,dst=/out" `
        --mount "type=volume,src=$workVolumeName,dst=/work,volume-nocopy" `
        --tmpfs '/tmp:exec' `
        $tag).Trim()
    if ($LASTEXITCODE -ne 0 -or $containerId -notmatch '^[a-f0-9]{64}$') {
        throw 'Phase 7A 容器创建失败'
    }
    $before = (& docker inspect $containerId | ConvertFrom-Json)[0]
    & docker start --attach $containerId
    $startExit = $LASTEXITCODE
    $after = (& docker inspect $containerId | ConvertFrom-Json)[0]
    $baseImage = (& docker image inspect $lock.toolchain.builder_image | ConvertFrom-Json)[0]
    $recipeImage = (& docker image inspect $tag | ConvertFrom-Json)[0]

    New-Item -ItemType Directory -Force -Path $evidenceRoot | Out-Null
    $mounts = @($before.Mounts | ForEach-Object {
        $resolvedMount = $_
        $configuredMount = @($before.HostConfig.Mounts | Where-Object { $_.Target -eq $resolvedMount.Destination }) | Select-Object -First 1
        [ordered]@{
            type = $resolvedMount.Type
            name = if ($resolvedMount.Type -eq 'volume') { $resolvedMount.Name } else { $null }
            destination = $resolvedMount.Destination
            readWrite = [bool]$resolvedMount.RW
            noCopy = [bool]($resolvedMount.Type -eq 'volume' -and $configuredMount.VolumeOptions.NoCopy)
        }
    } | Sort-Object destination)
    $hostEvidence = [ordered]@{
        schemaVersion = 2
        claim = 'one_locked_cold_build_candidate'
        lockSha256 = $lockSha256
        dockerBuild = [ordered]@{
            network = 'none'
            pull = $false
            context = 'desktop/third_party/mpv'
            dockerfile = 'desktop/third_party/mpv/build/Dockerfile'
        }
        baseImage = [ordered]@{
            reference = $lock.toolchain.builder_image
            id = $baseImage.Id
            repoDigests = @($baseImage.RepoDigests)
        }
        recipeImage = [ordered]@{
            tag = $tag
            id = $recipeImage.Id
        }
        workVolume = [ordered]@{
            name = $workVolume.Name
            driver = $workVolume.Driver
            scope = $workVolume.Scope
            createdAt = $workVolume.CreatedAt
            runId = $runId
            existedBeforeCreate = $false
            retained = $true
            labels = [ordered]@{
                runId = $workVolume.Labels.'com.autolive.phase7a.run-id'
                lockSha256 = $workVolume.Labels.'com.autolive.phase7a.lock-sha256'
                purpose = $workVolume.Labels.'com.autolive.phase7a.purpose'
            }
        }
        container = [ordered]@{
            id = $containerId
            name = $containerName
            user = $before.Config.User
            networkMode = $before.HostConfig.NetworkMode
            readonlyRootfs = [bool]$before.HostConfig.ReadonlyRootfs
            memoryBytes = [int64]$before.HostConfig.Memory
            memorySwapBytes = [int64]$before.HostConfig.MemorySwap
            entrypoint = @($before.Config.Entrypoint)
            command = @($before.Config.Cmd)
            mounts = $mounts
            startedAt = $after.State.StartedAt
            finishedAt = $after.State.FinishedAt
            exitCode = [int]$after.State.ExitCode
            status = $after.State.Status
            dockerStartExitCode = [int]$startExit
        }
    }
    $hostEvidence | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8NoBOM -LiteralPath (Join-Path $evidenceRoot 'docker-host-evidence.json')
    if ($startExit -ne 0 -or $after.State.ExitCode -ne 0) {
        throw "Phase 7A 断网冷构建失败，容器保留：$containerName"
    }

    & python -B (Join-Path $recipeRoot 'generate-evidence.py') `
        --report-only `
        --artifact-root $evidenceRoot `
        --output-root $OutputRoot `
        --lock $lockPath
    if ($LASTEXITCODE -ne 0) { throw 'Phase 7A 最终报告生成失败' }

    & node 'desktop/tools/verify-mpv-reproducible-build-report.mjs' `
        '--require-admitted' `
        '--lock' $lockPath `
        '--report' (Join-Path $OutputRoot 'reproducible-build-report.json') `
        '--evidence' $evidenceRoot `
        '--build-inputs' $inputRoot
    if ($LASTEXITCODE -ne 0) { throw "Phase 7A 语义门禁失败，容器保留：$containerName" }

    & (Join-Path $recipeRoot 'publish-build-evidence.ps1') -SourceRoot $OutputRoot -MpvRoot $mpvRoot
    if ($LASTEXITCODE -ne 0) { throw 'Phase 7A 证据事务发布失败' }
    Write-Output "Phase 7A locked cold-build candidate completed; container retained: $containerName"
} finally {
    Pop-Location
}
