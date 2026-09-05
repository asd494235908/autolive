[CmdletBinding()]
param(
    [string]$PackageRoot = (Join-Path $PSScriptRoot '..\artifacts\csharp-windows-controller-20260903-v89')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [OperatingSystem]::IsWindows()) {
    throw 'This C7 lock check only supports Windows.'
}

$mutexName = 'Local\GpAutoLive.CSharp.Windows.InstallTransaction.v1'
$mutex = [Threading.Mutex]::new($false, $mutexName)
try {
    try {
        $null = $mutex.WaitOne(0)
    }
    catch [AbandonedMutexException] {
        # The test process owns the recovered mutex and can continue.
    }

    $installRoot = Join-Path $PSScriptRoot '..\artifacts\c7-install-lock-test-root'
    $cases = @(
        @(
            (Join-Path $PSScriptRoot 'install-csharp-windows-package.ps1'),
            @('-PackageRoot', $PackageRoot, '-InstallRoot', $installRoot, '-Version', 'lock-test', '-WhatIf', '-Confirm:$false')),
        @(
            (Join-Path $PSScriptRoot 'rollback-csharp-windows-package.ps1'),
            @('-InstallRoot', $installRoot, '-WhatIf', '-Confirm:$false')),
        @(
            (Join-Path $PSScriptRoot 'uninstall-csharp-windows-package.ps1'),
            @('-InstallRoot', $installRoot, '-Version', 'lock-test', '-WhatIf', '-Confirm:$false'))
    )
    foreach ($case in $cases) {
        $output = & pwsh -NoProfile -NonInteractive -File $case[0] @($case[1]) 2>&1 | Out-String
        $exitCode = $LASTEXITCODE
        if ($exitCode -eq 0 -or $output -notmatch 'install_transaction_busy') {
            throw "Concurrent mutation was not rejected fail-closed (script=$($case[0]), exit=$exitCode)."
        }
    }
}
finally {
    try {
        $mutex.ReleaseMutex()
    }
    catch [ApplicationException] {
        # The test process may have recovered an abandoned mutex.
    }
    $mutex.Dispose()
}

Write-Output 'C7 install transaction lock check passed.'
