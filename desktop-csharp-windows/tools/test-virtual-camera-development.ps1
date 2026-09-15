[CmdletBinding()]
param([string]$PackageRoot)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $PackageRoot) { $PackageRoot = Join-Path $PSScriptRoot '../artifacts/virtual-camera-development' }
foreach ($script in @('build-virtual-camera-development.ps1','install-virtual-camera-development.ps1')) {
    $tokens = $null; $errors = $null
    [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $script),[ref]$tokens,[ref]$errors) | Out-Null
    if ($errors.Count) { throw ($errors | Out-String) }
}
$installer = Get-Content (Join-Path $PSScriptRoot 'install-virtual-camera-development.ps1') -Raw
if ([regex]::Matches($installer,'function Assert-DirectShowRegistration\(').Count -ne 1 -or [regex]::Matches($installer,'Invoke-Registration -Architecture \$arch -Remove \$false').Count -ne 1) { throw 'Registration and validator must each have one implementation.' }
$installBody = $installer.Substring($installer.IndexOf("if (`$Action -eq 'Uninstall') { `$registered"))
if ($installBody.IndexOf("@('add-format'") -gt $installBody.IndexOf('Invoke-Registration -Architecture $arch -Remove $false')) { throw 'DirectShow must register only after camera configuration exists.' }
if ($installer -match "Invoke-Manager -Arguments @\('update'\)") { throw 'Manager update cannot confirm DirectShow registration.' }
if ($installBody -notmatch 'Assert-DirectShowRegistration -Architecture \$arch -Present \$true') { throw 'Missing both-architecture registration verification.' }
$cleanup = $installer.Substring($installer.IndexOf('function Remove-OwnedRegistration'), $installer.IndexOf("if (`$Action -eq 'Uninstall') { `$registered") - $installer.IndexOf('function Remove-OwnedRegistration'))
if ($cleanup.IndexOf('Invoke-Registration') -gt $cleanup.IndexOf("@('remove-device'")) { throw 'Unregister DirectShow before deleting its camera configuration.' }
$root = Join-Path $PackageRoot 'akvirtualcamera'
$manifest = Get-Content (Join-Path $root 'development-manifest.json') -Raw | ConvertFrom-Json
if ($manifest.schemaVersion -ne 1 -or @($manifest.files.PSObject.Properties).Count -ne 6) { throw 'Invalid development manifest.' }
foreach ($item in $manifest.files.PSObject.Properties) {
    if ($item.Name -notmatch '^(bin/(akvirtualcamera-sidecar-x64\.exe|vcam_capi\.dll)|x64/(AkVirtualCamera\.dll|AkVCamAssistant\.exe|AkVCamManager\.exe)|x86/AkVirtualCamera\.dll)$') { throw 'Unexpected component path.' }
    $file = Join-Path $root $item.Name
    if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne $item.Value) { throw "Component hash mismatch: $($item.Name)" }
    $bytes = [IO.File]::ReadAllBytes($file)
    if ($bytes.Length -lt 64 -or [BitConverter]::ToUInt16($bytes,0) -ne 0x5a4d) { throw 'Missing executable DOS header.' }
    $offset = [BitConverter]::ToInt32($bytes,60)
    if ($offset -lt 64 -or $offset -gt ($bytes.Length - 6) -or [BitConverter]::ToUInt32($bytes,$offset) -ne 0x4550) { throw 'Missing PE header.' }
    $machine = if ($item.Name.StartsWith('x86/')) { 0x14c } else { 0x8664 }
    if ([BitConverter]::ToUInt16($bytes,$offset+4) -ne $machine) { throw "Wrong component architecture: $($item.Name)" }
}
if (Test-Path (Join-Path $root 'release-ready.json')) { throw 'Development output must never claim release readiness.' }
Write-Output 'PASS: installation order, dual-architecture verification, script syntax, six component hashes, PE x86/x64 architecture and development-only marker.'
