[CmdletBinding()]
param([Parameter(Mandatory)][string]$SourceDirectory,[Parameter(Mandatory)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$source = Get-Content (Join-Path $SourceDirectory 'windows/dshow/BaseFilter/src/pin.cpp') -Raw
$header = Get-Content (Join-Path $SourceDirectory 'windows/dshow/BaseFilter/src/pin.h') -Raw
if ($source -notmatch 'COM_INTERFACE\(IKsPropertySet\)' -or $header -notmatch 'public virtual IKsPropertySet') { throw 'Capture pin must expose IKsPropertySet.' }
$start = $source.IndexOf('// IKsPropertySet: capture category is read-only.')
$end = $source.IndexOf('HRESULT AkVCam::Pin::GetLatency', $start)
if ($start -lt 0 -or $end -le $start) { throw 'Pin category implementation missing.' }
$methods = $source.Substring($start,$end-$start).Replace('AkVCam::Pin::','')
$prefix = @'
#include <windows.h>
#include <dshow.h>
#include <ks.h>
#include <ksproxy.h>
#include <assert.h>
'@
$checks = @'
int main() {
    DWORD returned = 0, support = 0;
    GUID category = GUID_NULL;
    assert(Get(AMPROPSETID_Pin,AMPROPERTY_PIN_CATEGORY,nullptr,0,nullptr,0,&returned)==S_OK);
    assert(returned==sizeof(GUID));
    assert(Get(AMPROPSETID_Pin,AMPROPERTY_PIN_CATEGORY,nullptr,0,nullptr,0,nullptr)==E_POINTER);
    unsigned char shortBuffer[sizeof(GUID)]; memset(shortBuffer,0xA5,sizeof(shortBuffer));
    assert(FAILED(Get(AMPROPSETID_Pin,AMPROPERTY_PIN_CATEGORY,nullptr,0,shortBuffer,sizeof(GUID)-1,&returned)));
    for (auto byte: shortBuffer) assert(byte==0xA5);
    assert(Get(AMPROPSETID_Pin,AMPROPERTY_PIN_CATEGORY,nullptr,0,&category,sizeof(category),&returned)==S_OK);
    assert(category==PIN_CATEGORY_CAPTURE && returned==sizeof(GUID));
    assert(Get(AMPROPSETID_Pin,AMPROPERTY_PIN_CATEGORY,nullptr,0,&category,sizeof(category),nullptr)==S_OK);
    assert(Get(GUID_NULL,AMPROPERTY_PIN_CATEGORY,nullptr,0,&category,sizeof(category),&returned)==E_PROP_SET_UNSUPPORTED);
    assert(Get(AMPROPSETID_Pin,0xffffffff,nullptr,0,&category,sizeof(category),&returned)==E_PROP_ID_UNSUPPORTED);
    assert(QuerySupported(AMPROPSETID_Pin,AMPROPERTY_PIN_CATEGORY,&support)==S_OK && support==KSPROPERTY_SUPPORT_GET);
    assert(QuerySupported(AMPROPSETID_Pin,AMPROPERTY_PIN_CATEGORY,nullptr)==E_POINTER);
    assert(QuerySupported(GUID_NULL,AMPROPERTY_PIN_CATEGORY,&support)==E_PROP_SET_UNSUPPORTED && support==0);
    support=0xffffffff;
    assert(QuerySupported(AMPROPSETID_Pin,0xffffffff,&support)==E_PROP_ID_UNSUPPORTED && support==0);
    assert(Set(AMPROPSETID_Pin,AMPROPERTY_PIN_CATEGORY,nullptr,0,&category,sizeof(category))==E_NOTIMPL);
}
'@
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'pin_category_test.cpp'),$prefix + "`n" + $methods + $checks)
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'MSVC Build Tools required.' }
$build = '@echo off' + "`r`n" + 'call "' + (Join-Path $vs 'VC/Auxiliary/Build/vcvars64.bat') + '" >nul' + "`r`n" + 'cl /nologo /EHsc pin_category_test.cpp /link strmiids.lib /out:pin_category_test.exe'
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'build.cmd'),$build)
Push-Location $OutputDirectory
try {
    & './build.cmd'
    if ($LASTEXITCODE -ne 0) { throw 'Pin category regression compilation failed.' }
    & './pin_category_test.exe'
    if ($LASTEXITCODE -ne 0) { throw 'Pin category regression failed.' }
} finally { Pop-Location }
Write-Output 'PASS: actual pin category methods; size query, success, short-buffer protection, unsupported properties and read-only contract.'
