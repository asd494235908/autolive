[CmdletBinding()]
param([Parameter(Mandatory)][string]$SourceDirectory,[Parameter(Mandatory)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$service = Get-Content (Join-Path $SourceDirectory 'Service/src/service.cpp') -Raw
$server = Get-Content (Join-Path $SourceDirectory 'VCamUtils/src/messageserver.cpp') -Raw
$start = $service.IndexOf('void AkVCam::ServicePrivate::removeClientById(')
$end = $service.IndexOf('bool AkVCam::ServicePrivate::clients(', $start)
$remove = $service.Substring($start,$end-$start)
$start = $service.IndexOf('bool AkVCam::ServicePrivate::broadcast(')
$handlers = $service.Substring($start)
$start = $server.IndexOf('        Message outMessage;', $server.IndexOf('void AkVCam::MessageServerPrivate::connection('))
$end = $server.IndexOf('        if (!ok)', $start)
$dispatch = $server.Substring($start,$end-$start).Replace('        Message outMessage;','').Replace('{messageId, queryId, inData}','inMessage')
$prefix = @'
#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <functional>
#include <future>
#include <map>
#include <mutex>
#include <string>
#include <vector>
#include <stdio.h>
#define AkLogFunction() ((void)0)
#define AkLogDebug(...) ((void)0)
namespace AkVCam {
struct Message {
    int kind=0; std::string device="test"; uint64_t pid=0; int frame=0; bool active=false;
    uint64_t queryId() const { return 1; }
};
struct MsgListen {
    Message value; MsgListen(const Message& v):value(v) {}
    const std::string& device() const { return value.device; }
    uint64_t pid() const { return value.pid; }
};
struct MsgBroadcast: MsgListen {
    using MsgListen::MsgListen; int frame() const { return value.frame; }
};
struct MsgStatus {
    int code; MsgStatus(int v,uint64_t):code(v) {}
    int status() const { return code; } Message toMessage() const { Message v; v.frame=code; return v; }
};
struct MsgFrameReady {
    Message value;
    MsgFrameReady(const std::string& device,int frame,bool active,uint64_t) { value.device=device;value.frame=frame;value.active=active; }
    Message toMessage() const { return value; }
};
struct Peer { uint64_t clientId=0,pid=0; };
struct BroadcastSlot { Peer broadcaster; std::vector<Peer> listeners; int frame=0; bool frameReady=false; };
struct ObservedCondition {
    std::condition_variable_any condition; std::promise<void> entered;
    template<class L,class D> void wait_for(L& lock,D duration) { entered.set_value(); condition.wait_for(lock,duration); }
    void notify_all() { condition.notify_all(); }
};
class ServicePrivate {
public:
    std::map<std::string,BroadcastSlot> m_broadcasts;
    ObservedCondition m_frameAvailable; std::mutex m_peerMutex;
    static void removeClientById(void*,uint64_t);
    bool broadcast(uint64_t,const Message&,Message&);
    bool listen(uint64_t,const Message&,Message&);
};
class MessageServer { public: using MessageHandler=std::function<bool(uint64_t,const Message&,Message&)>; };
class MessageServerPrivate {
public:
    std::map<int,MessageServer::MessageHandler> m_handlers; std::mutex m_handlersMutex;
    bool dispatch(uint64_t clientId,const Message& inMessage,Message& outMessage) {
        int messageId=inMessage.kind; bool ok=true;
'@
$middle = @'
        return ok;
    }
};
}
'@
$checks = @'
int main() {
    using namespace AkVCam; using namespace std::chrono_literals;
    bool success=true;
    {
        ServicePrivate service; MessageServerPrivate server;
        server.m_handlers[1]=[&](uint64_t id,const Message& in,Message& out){return service.listen(id,in,out);};
        server.m_handlers[2]=[&](uint64_t id,const Message& in,Message& out){return service.broadcast(id,in,out);};
        Message listen{1,"test",10}, broadcast{2,"test",20,42}, received, status;
        auto entered=service.m_frameAvailable.entered.get_future();
        auto listener=std::async(std::launch::async,[&]{return server.dispatch(1,listen,received);});
        if(entered.wait_for(2s)!=std::future_status::ready) return 2;
        auto writer=std::async(std::launch::async,[&]{return server.dispatch(2,broadcast,status);});
        bool timely=writer.wait_for(500ms)==std::future_status::ready;
        writer.get(); listener.get();
        bool passed=timely && received.frame==42 && received.active;
        printf("handler_wait_releases_dispatch_lock=%d\n",passed); success &= passed;
    }
    {
        ServicePrivate service; Message listen{1,"test",10}, received;
        for(int i=0;i<3;i++){service.m_broadcasts["test"].frameReady=true;service.listen(1,listen,received);}
        bool passed=service.m_broadcasts["test"].listeners.size()==1;
        printf("listener_identity_unique=%d\n",passed); success &= passed;
    }
    {
        ServicePrivate service; service.m_broadcasts["test"].broadcaster={2,20};
        service.m_broadcasts["test"].frame=42;
        Message listen{1,"test",10}, received;
        auto entered=service.m_frameAvailable.entered.get_future();
        auto listener=std::async(std::launch::async,[&]{return service.listen(1,listen,received);});
        if(entered.wait_for(2s)!=std::future_status::ready) return 2;
        ServicePrivate::removeClientById(&service,2);
        bool timely=listener.wait_for(500ms)==std::future_status::ready;
        listener.get(); bool passed=timely && !received.active && received.frame==0;
        printf("disconnect_wakes_inactive_listener=%d\n",passed); success &= passed;
    }
    return success? 0: 1;
}
'@
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'service_wakeup_test.cpp'),$prefix + "`n" + $dispatch + $middle + $remove + $handlers + $checks)
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'MSVC Build Tools required.' }
$build = '@echo off' + "`r`n" + 'call "' + (Join-Path $vs 'VC/Auxiliary/Build/vcvars64.bat') + '" >nul' + "`r`n" + 'cl /nologo /EHsc /std:c++17 service_wakeup_test.cpp /Fe:service_wakeup_test.exe'
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'build.cmd'),$build)
Push-Location $OutputDirectory
try {
    & './build.cmd'
    if ($LASTEXITCODE -ne 0) { throw 'Service wakeup regression compilation failed.' }
    & './service_wakeup_test.exe'
    if ($LASTEXITCODE -ne 0) { throw 'Service wakeup regression failed.' }
} finally { Pop-Location }
Write-Output 'PASS: actual dispatch/service handlers allow broadcast wakeup, deduplicate listeners and publish inactive state on disconnect.'
