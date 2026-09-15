[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Assert-Contains([string]$Text, [string]$Pattern, [string]$Message) {
    if ($Text -notmatch $Pattern) {
        throw $Message
    }
}

$sourcePath = Join-Path $PSScriptRoot '..\installer\windows\GpAutoLive.nsi'
$builderPath = Join-Path $PSScriptRoot 'build-csharp-windows-nsis.ps1'
$source = Get-Content -LiteralPath $sourcePath -Raw
$builder = Get-Content -LiteralPath $builderPath -Raw

Assert-Contains $source 'RequestExecutionLevel admin' 'Installer must request elevation for the offline .NET runtime.'
Assert-Contains $source 'IntCmp \$0 19041 supported_system unsupported_system supported_system' 'Installer must reject Windows builds older than 19041.'
Assert-Contains $source 'windowsdesktop-runtime-10\.0\.11-win-x64\.exe' 'Installer must embed the pinned .NET Desktop Runtime.'
Assert-Contains $source 'ExecWait.+/install /quiet /norestart' 'Installer must execute the offline runtime with bounded official switches.'
Assert-Contains $source 'CreateShortcut.+\$DESKTOP' 'Installer must create a desktop shortcut.'
Assert-Contains $source 'CreateShortcut.+\$SMPROGRAMS' 'Installer must create Start Menu shortcuts.'
Assert-Contains $source 'WriteRegStr HKLM.+UninstallString' 'Installer must register a Windows uninstall entry.'
Assert-Contains $source 'GpAutoLive\.control-plane-profile' 'Installer must write an app-scoped control-plane package profile.'
Assert-Contains $source 'FileWrite.+\$\{CONTROL_PLANE_PROFILE\}' 'Installer must write the selected immutable package profile.'
Assert-Contains $source 'Rename.+\.staging-' 'Installer must activate from a same-volume staging directory.'
Assert-Contains $source '(?s)File /r.+SetOutPath "\$INSTDIR".+Rename "\$INSTDIR\\\.staging-' 'Installer must leave the staging directory before renaming it into place.'
Assert-Contains $source 'Rename.+\.rollback.+\\app' 'Installer must restore the previous application directory when activation fails.'
Assert-Contains $source 'CreateFileW\(w "\$\{APP_EXE\}".+0x00010000' 'Installer must reject an executable held open by another user session.'
Assert-Contains $source 'Call un\.EnsureAppStopped' 'Uninstall must reject a running application.'
Assert-Contains $source 'Rename "\$INSTDIR\\app" "\$INSTDIR\\\.uninstall"' 'Uninstall must first isolate the installer-owned payload.'
Assert-Contains $source '(?s)registration_failed:.+DeleteRegKey.+RMDir /r "\$INSTDIR\\app".+Rename "\$INSTDIR\\\.rollback" "\$INSTDIR\\app"' 'Failed shell registration must restore the previous application payload.'
if ($source -match '(?i)webview2|powershell|pwsh') {
    throw 'The C# WPF first-install path must not depend on WebView2 or PowerShell.'
}
if ($source -match 'MUI_FINISHPAGE_RUN') {
    throw 'The elevated installer must not launch the desktop application with administrator privileges.'
}
Assert-Contains $builder 'Get-FileHash.+SHA512' 'Builder must verify the pinned runtime SHA-512.'
Assert-Contains $builder 'Get-AuthenticodeSignature.+runtime' 'Builder must verify the Microsoft runtime signature.'
Assert-Contains $builder 'RequireReleaseReadyLegal' 'Formal builder mode must enforce release-ready legal review.'
Assert-Contains $builder 'REQUIRES-OUTER-SIGNATURE' 'A compiled but unsigned formal installer must remain visibly marked for outer signing.'
Assert-Contains $builder "ValidateSet\('Production', 'CloudTest'\)" 'Builder must expose explicit production and cloud-test package profiles.'
Assert-Contains $builder 'CloudTest.+DevelopmentUnsigned' 'A cloud-test package must remain an explicitly development-only artifact.'
Assert-Contains $builder '/DCONTROL_PLANE_PROFILE=' 'Package compilation must embed its app-scoped profile marker.'
Assert-Contains $builder 'http://101\.96\.208\.132:9090' 'Cloud-test package reports must identify the fixed cloud endpoint.'
if ($source -match 'Session Manager\\Environment' -or $source -match 'HKCU.+Environment') {
    throw 'Installer must not write process configuration into global Windows environment variables.'
}

Write-Output 'C7 NSIS source boundary check passed.'
