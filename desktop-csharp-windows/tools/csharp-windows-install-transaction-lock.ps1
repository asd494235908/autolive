Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Install/rollback/uninstall each run in a separate PowerShell process. Keeping
# this handle alive until process exit releases the OS mutex on every exit path.
$csharpWindowsInstallTransactionMutexName = 'Local\GpAutoLive.CSharp.Windows.InstallTransaction.v1'
$csharpWindowsInstallTransactionMutex = $null
try {
    $csharpWindowsInstallTransactionMutex = [Threading.Mutex]::new(
        $false,
        $csharpWindowsInstallTransactionMutexName)
    try {
        $csharpWindowsInstallTransactionAcquired = $csharpWindowsInstallTransactionMutex.WaitOne(0)
    }
    catch [AbandonedMutexException] {
        $csharpWindowsInstallTransactionAcquired = $true
    }

    if (-not $csharpWindowsInstallTransactionAcquired) {
        throw [InvalidOperationException]::new('install_transaction_busy')
    }
}
catch {
    if ($null -ne $csharpWindowsInstallTransactionMutex) {
        $csharpWindowsInstallTransactionMutex.Dispose()
    }
    $csharpWindowsInstallTransactionMutex = $null
    throw
}
