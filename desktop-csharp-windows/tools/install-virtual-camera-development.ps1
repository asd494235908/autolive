[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$PackageRoot,
    [ValidateSet('Install','Uninstall')][string]$Action = 'Install',
    [string]$DeveloperSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
if (-not ([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Administrator PowerShell is required to register DirectShow. Re-run this script elevated; no elevation bypass is attempted.'
}
$sid = [Security.Principal.SecurityIdentifier]::new($DeveloperSid)
if (-not $DeveloperSid.StartsWith('S-1-5-21-')) { throw 'DeveloperSid must identify one local/domain user, not an everyone/group grant.' }
$root = [IO.Path]::GetFullPath((Join-Path $PackageRoot 'akvirtualcamera')).TrimEnd('\')
if ($root.StartsWith('\\')) { throw 'Development components must be on a local drive.' }
function Assert-LocalRegularPath([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force
    while ($item) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Reparse points are not allowed in an elevated component path.' }
        $parent = Split-Path -Parent $item.FullName
        if (-not $parent -or $parent -eq $item.FullName) { break }
        $item = Get-Item -LiteralPath $parent -Force
    }
}
Assert-LocalRegularPath -Path $root
Assert-LocalRegularPath -Path (Join-Path $root 'development-manifest.json')
$names = @('bin/akvirtualcamera-sidecar-x64.exe','bin/vcam_capi.dll','x64/AkVirtualCamera.dll','x64/AkVCamAssistant.exe','x64/AkVCamManager.exe','x86/AkVirtualCamera.dll')
$manifest = Get-Content -LiteralPath (Join-Path $root 'development-manifest.json') -Raw | ConvertFrom-Json
if ($manifest.schemaVersion -ne 1 -or @($manifest.files.PSObject.Properties).Count -ne $names.Count) { throw 'Invalid development manifest.' }
foreach ($name in $names) {
    Assert-LocalRegularPath -Path (Join-Path $root $name)
    $hash = $manifest.files.PSObject.Properties[$name].Value
    if ($hash -notmatch '^[a-fA-F0-9]{64}$' -or (Get-FileHash -LiteralPath (Join-Path $root $name) -Algorithm SHA256).Hash -ne $hash) { throw "Development hash mismatch: $name" }
}
$keyPath = 'SOFTWARE\Webcamoid\VirtualCamera'
$views = @([Microsoft.Win32.RegistryView]::Registry64,[Microsoft.Win32.RegistryView]::Registry32)
$manager = Join-Path $root 'x64/AkVCamManager.exe'
function Invoke-Manager([string[]]$Arguments) {
    $quoted = $Arguments | ForEach-Object { '"' + $_ + '"' }
    $process = Start-Process -FilePath $manager -ArgumentList $quoted -WindowStyle Hidden -PassThru
    try {
        if (-not $process.WaitForExit(30000)) { $process.Kill(); throw 'AkVCamManager timed out.' }
        if ($process.ExitCode -ne 0) { throw "AkVCamManager failed: $($Arguments[0]) ($($process.ExitCode))" }
    } finally { $process.Dispose() }
}
function Invoke-Registration([string]$Architecture,[bool]$Remove) {
    $regsvr = if ($Architecture -eq 'x64') { "$env:WINDIR/System32/regsvr32.exe" } else { "$env:WINDIR/SysWOW64/regsvr32.exe" }
    $arguments = @('/s')
    if ($Remove) { $arguments += '/u' }
    $arguments += '"' + (Join-Path $root "$Architecture/AkVirtualCamera.dll") + '"'
    $process = Start-Process -FilePath $regsvr -ArgumentList $arguments -WindowStyle Hidden -PassThru
    try {
        if (-not $process.WaitForExit(30000)) { $process.Kill(); throw 'DirectShow registration timed out.' }
        if ($process.ExitCode -ne 0) { throw "DirectShow registration failed: $Architecture ($($process.ExitCode))" }
    } finally { $process.Dispose() }
}
function Assert-DirectShowRegistration([string]$Architecture,[bool]$Present) {
    $view = if ($Architecture -eq 'x64') { [Microsoft.Win32.RegistryView]::Registry64 } else { [Microsoft.Win32.RegistryView]::Registry32 }
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::ClassesRoot,$view)
    try {
        $category = $base.OpenSubKey('CLSID\{860BB310-5D01-11D0-BD3B-00A0C911CE86}\Instance')
        $count = 0
        if ($category) {
            try {
                foreach ($name in $category.GetSubKeyNames()) {
                    $entry = $category.OpenSubKey($name)
                    try {
                        if ($entry.GetValue('DevicePath','') -ne 'GpAutoLiveCamera') { continue }
                        $count++
                        if (-not $Present) { throw "DirectShow removal incomplete: $Architecture" }
                        if ($entry.GetValue('FriendlyName','') -ne 'GpAutoLive Camera') { throw 'DirectShow identity mismatch.' }
                        $clsid = [guid]::Parse([string]$entry.GetValue('CLSID','')).ToString('B')
                        $server = $base.OpenSubKey("CLSID\$clsid\InprocServer32")
                        try {
                            if (-not $server -or [IO.Path]::GetFullPath([string]$server.GetValue('','')) -ne (Join-Path $root "$Architecture/AkVirtualCamera.dll")) { throw "DirectShow component ownership mismatch: $Architecture" }
                        } finally { if ($server) { $server.Dispose() } }
                    } finally { $entry.Dispose() }
                }
            } finally { $category.Dispose() }
        }
        if ($Present -and $count -ne 1) { throw "DirectShow device missing or duplicated: $Architecture" }
    } finally { $base.Dispose() }
}
foreach ($view in $views) {
    $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine,$view)
    try {
        $key = $base.OpenSubKey($keyPath)
        if ($key) {
            try {
                $owner = [string]$key.GetValue('installPath','')
                if ($owner -and $owner -ne $root) { throw 'Another AkVirtualCamera owner is installed; do not overwrite it.' }
                if (-not $owner -and ($key.GetSubKeyNames().Count -gt 0 -or @($key.GetValueNames() | Where-Object { $_ -ne 'installPath' }).Count -gt 0)) {
                    throw 'Existing AkVirtualCamera configuration has no verified owner; refusing to alter it.'
                }
                if ($Action -eq 'Install' -and $owner) { throw 'Development package is already registered. Close consumers and uninstall it before reinstalling.' }
                if ($Action -eq 'Uninstall' -and $owner -ne $root) { throw 'Cannot uninstall a device not owned by this package.' }
            } finally { $key.Dispose() }
        } elseif ($Action -eq 'Uninstall') { throw 'Development device is not installed.' }
    } finally { $base.Dispose() }
}
$registered = @(); $deviceAdded = $false
function Remove-OwnedRegistration {
    foreach ($arch in $registered) {
        Invoke-Registration -Architecture $arch -Remove $true
        Assert-DirectShowRegistration -Architecture $arch -Present $false
    }
    if ($deviceAdded -or $Action -eq 'Uninstall') { Invoke-Manager -Arguments @('remove-device','GpAutoLiveCamera') }
    foreach ($view in $views) {
        $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine,$view)
        try { $key = $base.OpenSubKey($keyPath,$true); if ($key) { try { $key.DeleteValue('installPath',$false) } finally { $key.Dispose() } } } finally { $base.Dispose() }
    }
}
if ($Action -eq 'Uninstall') { $registered = @('x64','x86'); Remove-OwnedRegistration; return }
try {
    foreach ($view in $views) {
        $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine,$view)
        try { $key = $base.CreateSubKey($keyPath); try { $key.SetValue('installPath',$root) } finally { $key.Dispose() } } finally { $base.Dispose() }
    }

    Invoke-Manager -Arguments @('add-device','-i','GpAutoLiveCamera','GpAutoLive Camera'); $deviceAdded = $true
    Invoke-Manager -Arguments @('remove-formats','GpAutoLiveCamera')
    Invoke-Manager -Arguments @('add-format','GpAutoLiveCamera','YUY2','1280','720','30')
    Invoke-Manager -Arguments @('set-data-mode','mmap')
    Invoke-Manager -Arguments @('set-direct-mode','GpAutoLiveCamera','1')
    foreach ($arch in @('x64','x86')) {
        $registered += $arch
        Invoke-Registration -Architecture $arch -Remove $false
    }
    foreach ($arch in @('x64','x86')) { Assert-DirectShowRegistration -Architecture $arch -Present $true }
    # Both x86 and x64 upstream filters explicitly read the 64-bit preferences view.
    foreach ($view in @([Microsoft.Win32.RegistryView]::Registry64)) {
        $base = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine,$view)
        try {
            $cameras = $base.OpenSubKey("$keyPath\Cameras")
            if (-not $cameras) { throw 'Camera registration missing.' }
            try {
                $found = $false
                foreach ($index in $cameras.GetSubKeyNames()) {
                    $camera = $cameras.OpenSubKey($index)
                    try {
                        if ($camera.GetValue('id') -ne 'GpAutoLiveCamera') { continue }
                        if ($camera.GetValue('description') -ne 'GpAutoLive Camera') { throw 'Camera identity mismatch.' }
                        $format = $base.OpenSubKey("$keyPath\Cameras\$index\Formats\1",[Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree,([Security.AccessControl.RegistryRights]::ChangePermissions -bor [Security.AccessControl.RegistryRights]::ReadPermissions))
                        try {
                            $acl = $format.GetAccessControl()
                            $acl.AddAccessRule([Security.AccessControl.RegistryAccessRule]::new($sid,[Security.AccessControl.RegistryRights]::SetValue,[Security.AccessControl.AccessControlType]::Allow))
                            $format.SetAccessControl($acl); $found = $true
                        } finally { $format.Dispose() }
                    } finally { $camera.Dispose() }
                }
                if (-not $found) { throw 'Owned format key missing.' }
            } finally { $cameras.Dispose() }
        } finally { $base.Dispose() }
    }
} catch {
    $failure = $_
    try { Remove-OwnedRegistration } catch { throw "Install failed; rollback also failed: $($_.Exception.Message). Original: $($failure.Exception.Message)" }
    throw $failure
}
Write-Output 'Development device installed. Close downstream consumers before changing source resolution; reopen them after output starts. No release readiness is implied.'


