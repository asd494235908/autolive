[CmdletBinding()]
param([Parameter(Mandatory)][string]$SourceDirectory,[Parameter(Mandatory)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$SourceDirectory = [IO.Path]::GetFullPath($SourceDirectory)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
# Run the real MessageClient and Sockets implementations with only message/log
# payload dependencies replaced. The network, deadlines, async launch and cleanup
# are not mocked. No registered device or running Assistant is contacted.
$source = Get-Content (Join-Path $SourceDirectory 'VCamUtils/src/messageclient.cpp') -Raw
foreach ($header in @('logger.h','message.h','utils.h')) { $source = $source.Replace('#include "' + $header + '"','#include "message_fixture.h"') }
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'messageclient_tested.cpp'),$source)
$fixture = @'
#pragma once
#include <cstdint>
#include <string>
#include <vector>
#include <system_error>
#define AkLogFunction() ((void)0)
#define AkLogDebug(...) ((void)0)
#define AkLogError(...) ((void)0)
#define AkLogCritical(...) ((void)0)
#define UNUSED(x) (void)(x)
namespace AkVCam {
inline uint64_t id() { return 1; }
inline std::string stringFromMessageId(int) { return {}; }
class Message {
    int m_id; uint64_t m_query; std::vector<char> m_data;
public:
    Message(int id=0,uint64_t query=0,const std::vector<char>& data={}):m_id(id),m_query(query),m_data(data) {}
    int id() const { return m_id; }
    uint64_t queryId() const { return m_query; }
    const std::vector<char>& data() const { return m_data; }
};
}
'@
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'message_fixture.h'),$fixture)
$test = @'
#include "message_fixture.h"
#include "messageclient.h"
#include "sockets.h"
#include <chrono>
#include <assert.h>
#include <stdio.h>
using namespace AkVCam;
using namespace std::chrono_literals;
int main() {
    for (int mode: {0,1,2,3}) {
        MessageClient client;
        auto listener=socket(AF_INET,SOCK_STREAM,0);
        assert(listener!=INVALID_SOCKET);
        sockaddr_in address{}; address.sin_family=AF_INET;
        address.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
        assert(bind(listener,reinterpret_cast<sockaddr*>(&address),sizeof(address))==0);
        int size=sizeof(address);
        assert(getsockname(listener,reinterpret_cast<sockaddr*>(&address),&size)==0);
        assert(listen(listener,1)==0);
        client.setPort(ntohs(address.sin_port));
        std::promise<void> seen, release;
        auto seenFuture=seen.get_future(); auto releaseFuture=release.get_future();
        auto server=std::async(std::launch::async,[&] {
            auto peer=accept(listener,nullptr,nullptr); assert(peer!=INVALID_SOCKET);
            if(mode==3) { seen.set_value(); releaseFuture.wait(); }
            else {
                int id=0; uint64_t query=0; std::vector<char> data;
                assert(Sockets::recv(peer,id) && Sockets::recv(peer,query) && Sockets::recv(peer,data));
                seen.set_value();
                if(mode==0) {
                    assert(Sockets::send(peer,id) && Sockets::send(peer,query) && Sockets::send(peer,data));
                } else if(mode==2) {
                    assert(Sockets::send(peer,id));
                    // A deliberately slow peer splits the reply across the same
                    // request budget; receiving queryId must not reset its 5s.
                    if(releaseFuture.wait_for(3500ms)!=std::future_status::ready) {
                        Sockets::send(peer,query); releaseFuture.wait();
                    }
                } else releaseFuture.wait();
            }
            Sockets::closeSocket(peer);
        });
        auto cancel=std::make_shared<std::atomic_bool>(false);
        std::vector<char> payload(mode==3? 16*1024*1024: 2,'x');
        Message request{9,123,payload};
        bool responseMatched=false;
        auto started=std::chrono::steady_clock::now();
        auto future=client.send(request,[&](const Message& response) {
            responseMatched=response.id()==9 && response.queryId()==123 && response.data()==payload;
            return false;
        },cancel);
        assert(seenFuture.wait_for(2s)==std::future_status::ready);
        if(mode==1 || mode==3) cancel->store(true);
        auto ready=future.wait_for(mode==2? 6s: 500ms)==std::future_status::ready;
        release.set_value(); server.get(); Sockets::closeSocket(listener);
        assert(ready); bool success=future.get();
        auto elapsed=std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now()-started).count();
        if(mode==0) assert(success && responseMatched);
        else assert(!success && !responseMatched);
        if(mode==2) assert(elapsed>=4500 && elapsed<6000);
        printf("mode=%d elapsed_ms=%lld joined=true\n",mode,(long long)elapsed);
    }
}
'@
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'socket_lifecycle_test.cpp'),$test)
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'MSVC Build Tools required.' }
$utils = Join-Path $SourceDirectory 'VCamUtils/src'
$build = '@echo off' + "`r`n" + 'call "' + (Join-Path $vs 'VC/Auxiliary/Build/vcvars64.bat') + '" >nul' + "`r`n" + 'cl /nologo /EHsc /std:c++17 /I"' + $utils + '" socket_lifecycle_test.cpp messageclient_tested.cpp "' + (Join-Path $utils 'sockets.cpp') + '" /link ws2_32.lib /out:socket_lifecycle_test.exe'
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'build.cmd'),$build)
Push-Location $OutputDirectory
try {
    & './build.cmd'
    if ($LASTEXITCODE -ne 0) { throw 'Socket lifecycle regression compilation failed.' }
    & './socket_lifecycle_test.exe'
    if ($LASTEXITCODE -ne 0) { throw 'Socket lifecycle regression failed.' }
} finally { Pop-Location }
Write-Output 'PASS: real local request/reply, receive/send cancellation, whole-reply deadline and joined workers.'
