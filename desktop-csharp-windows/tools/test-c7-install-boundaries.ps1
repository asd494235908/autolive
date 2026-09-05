[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [OperatingSystem]::IsWindows()) {
    throw 'This C7 boundary check only supports Windows.'
}

function Assert-Condition([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw $Message
    }
}

function New-TestEnvironment {
    $root = Join-Path ([IO.Path]::GetTempPath()) ('gpautolive-c7-boundaries-' + [Guid]::NewGuid().ToString('N'))
    $tools = Join-Path $root 'tools'
    $package = Join-Path $root 'package'
    New-Item -ItemType Directory -Path $tools, $package | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'install-csharp-windows-package.ps1') -Destination $tools
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'csharp-windows-install-transaction-lock.ps1') -Destination $tools
    @'
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$PackageRoot)
Write-Output '{"schema_version":1,"package_root_files":[{}],"gpu_manifest_files":[{}],"winrt_manifest_files":[{}],"media_manifest_files":[{}]}'
'@ | Set-Content -LiteralPath (Join-Path $tools 'verify-release-package.ps1') -Encoding utf8
    Set-Content -LiteralPath (Join-Path $package 'payload.txt') -Value 'payload' -Encoding utf8
    [pscustomobject]@{ Root = $root; Tools = $tools; Package = $package }
}

$environment = New-TestEnvironment
try {
    $install = Join-Path $environment.Root 'install-invalid-pointer'
    New-Item -ItemType Directory -Path (Join-Path $install 'versions') -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $install 'current.json') -Value '{"schema_version":1,"active_version":"../escape","relative_path":"versions/../escape","previous_version":null}' -Encoding utf8
    $null = & pwsh -NoProfile -NonInteractive -File (Join-Path $environment.Tools 'install-csharp-windows-package.ps1') `
        -PackageRoot $environment.Package -InstallRoot $install -Version 'new-version' -WhatIf -Confirm:$false 2>&1 | Out-String
    Assert-Condition ($LASTEXITCODE -ne 0) 'WhatIf must reject a malformed existing current.json.'
    Assert-Condition (-not (Test-Path -LiteralPath (Join-Path $install 'versions/new-version'))) 'Malformed pointer must not create a target version.'

    $install = Join-Path $environment.Root 'install-whatif'
    $null = & pwsh -NoProfile -NonInteractive -File (Join-Path $environment.Tools 'install-csharp-windows-package.ps1') `
        -PackageRoot $environment.Package -InstallRoot $install -Version 'preview-version' -WhatIf -Confirm:$false 2>&1 | Out-String
    Assert-Condition ($LASTEXITCODE -eq 0) 'A valid WhatIf install preview must succeed.'
    Assert-Condition (-not (Test-Path -LiteralPath $install)) 'WhatIf must not create the installation root.'

    $install = Join-Path $environment.Root 'install-pointer-failure'
    New-Item -ItemType Directory -Path (Join-Path $install 'versions/old-version') -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $install 'versions/old-version/payload.txt') -Value 'old' -Encoding utf8
    Set-Content -LiteralPath (Join-Path $install 'current.json') -Value '{"schema_version":1,"active_version":"old-version","relative_path":"versions/old-version","previous_version":null}' -Encoding utf8
    $currentPath = Join-Path $install 'current.json'
    $before = Get-Content -LiteralPath $currentPath -Raw
    (Get-Item -LiteralPath $currentPath).IsReadOnly = $true
    $null = & pwsh -NoProfile -NonInteractive -File (Join-Path $environment.Tools 'install-csharp-windows-package.ps1') `
        -PackageRoot $environment.Package -InstallRoot $install -Version 'new-version' -Confirm:$false 2>&1 | Out-String
    Assert-Condition ($LASTEXITCODE -ne 0) 'Pointer activation failure must fail the install.'
    Assert-Condition (-not (Test-Path -LiteralPath (Join-Path $install 'versions/new-version'))) 'Failed pointer activation must remove the staged target version.'
    Assert-Condition ((Get-Content -LiteralPath $currentPath -Raw) -eq $before) 'Failed pointer activation must preserve current.json.'
}
finally {
    if (Test-Path -LiteralPath $environment.Root) {
        Get-ChildItem -LiteralPath $environment.Root -Force -Recurse -File -ErrorAction SilentlyContinue | ForEach-Object { $_.IsReadOnly = $false }
        Remove-Item -LiteralPath $environment.Root -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Write-Output 'C7 install boundary check passed.'
