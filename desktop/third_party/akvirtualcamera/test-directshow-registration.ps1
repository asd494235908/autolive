[CmdletBinding()]
param([Parameter(Mandatory)][string]$SourceDirectory,[Parameter(Mandatory)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
# Compile the actual upstream methods, replacing only registry-mutating calls.
$source = Get-Content (Join-Path $SourceDirectory 'windows/dshow/VirtualCamera/src/plugininterface.cpp') -Raw
$start = $source.IndexOf('bool AkVCam::PluginInterfacePrivate::registerFilter(')
$end = $source.IndexOf('void AkVCam::PluginInterfacePrivate::unregisterFilter(const std::string', $start)
$register = $source.Substring($start,$end-$start).Replace('AkVCam::PluginInterfacePrivate::registerFilter','registerFilter').Replace(') const',')')
$start = $source.IndexOf('void AkVCam::PluginInterfacePrivate::unregisterFilter(const CLSID &clsid)')
$end = $source.IndexOf('bool AkVCam::PluginInterfacePrivate::setDeviceId', $start)
$unregister = $source.Substring($start,$end-$start).Replace('AkVCam::PluginInterfacePrivate::unregisterFilter','unregisterFilter').Replace(') const',')')
$register = [regex]::Replace($register,'filterMapper->RegisterFilter\([\s\S]*?\);','injectedResult;')
$unregister = [regex]::Replace($unregister,'filterMapper->UnregisterFilter\([\s\S]*?\);','S_OK;')
$prefix = @'
#include <windows.h>
#include <dshow.h>
#include <vector>
#include <string>
#include <thread>
#include <assert.h>
#define AkLogFunction() ((void)0)
#define AkLogError(...) ((void)0)
#define AkLogInfo(...) ((void)0)
namespace AkVCam { CLSID createClsidFromStr(const std::string&) { return GUID_NULL; } }
LPWSTR wstrFromString(const std::string&) { return static_cast<LPWSTR>(CoTaskMemAlloc(2)); }
HRESULT injectedResult = S_OK;
'@
$suffix = @'
void check(int mode) {
    if (mode >= 0) assert(SUCCEEDED(CoInitializeEx(nullptr, mode)));
    APTTYPE before; APTTYPEQUALIFIER qualifier;
    const HRESULT initial = CoGetApartmentType(&before, &qualifier);
    injectedResult = E_ACCESSDENIED;
    assert(!registerFilter("test", "test"));
    injectedResult = S_OK;
    assert(registerFilter("test", "test"));
    unregisterFilter(GUID_NULL);
    APTTYPE after;
    assert(CoGetApartmentType(&after, &qualifier) == initial);
    if (SUCCEEDED(initial)) assert(after == before);
    if (mode >= 0) {
        CoUninitialize();
        assert(CoGetApartmentType(&after, &qualifier) == CO_E_NOTINITIALIZED);
    }
}
int main() {
    for (int mode : {int(COINIT_APARTMENTTHREADED), int(COINIT_MULTITHREADED), -1}) {
        std::thread worker([mode] { check(mode); }); worker.join();
    }
}
'@
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'registration_test.cpp'),$prefix + "`n" + $register + $unregister + $suffix)
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'MSVC Build Tools required.' }
$build = '@echo off' + "`r`n" + 'call "' + (Join-Path $vs 'VC/Auxiliary/Build/vcvars64.bat') + '" >nul' + "`r`n" + 'cl /nologo /EHsc /std:c++17 registration_test.cpp /link ole32.lib strmiids.lib /out:registration_test.exe'
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'build.cmd'),$build)
Push-Location $OutputDirectory
try {
    & './build.cmd'
    if ($LASTEXITCODE -ne 0) { throw 'Registration regression compilation failed.' }
    & './registration_test.exe'
    if ($LASTEXITCODE -ne 0) { throw 'Registration COM apartment/result regression failed.' }
} finally { Pop-Location }
Write-Output 'PASS: actual registration methods preserve uninitialized/STA/MTA COM lifetime and reject RegisterFilter failure; no registry writes.'
