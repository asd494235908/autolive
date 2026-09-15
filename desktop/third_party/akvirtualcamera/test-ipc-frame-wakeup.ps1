[CmdletBinding()]
param([Parameter(Mandatory)][string]$SourceDirectory,[Parameter(Mandatory)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$source = Get-Content (Join-Path $SourceDirectory 'windows/VCamIPC/src/ipcbridge.cpp') -Raw
$start = $source.IndexOf('bool AkVCam::IpcBridgePrivate::frameRequired(')
$end = $source.IndexOf('bool AkVCam::IpcBridgePrivate::frameReady(', $start)
$callback = $source.Substring($start,$end-$start)
$start = $source.IndexOf('void AkVCam::IpcBridge::deviceStop(')
$end = $source.IndexOf('bool AkVCam::IpcBridge::write(', $start)
$stop = $source.Substring($start,$end-$start)
$prefix = @'
#include <chrono>
#include <atomic>
#include <memory>
#include <condition_variable>
#include <future>
#include <map>
#include <mutex>
#include <string>
#include <stdio.h>
#define AkLogFunction() ((void)0)
#define AkLogDebug(...) ((void)0)
#define AkLogWarning(...) ((void)0)
#define AkLogError(...) ((void)0)
namespace AkVCam {
struct Message { int frame = 0; };
struct MsgBroadcast {
    int frame;
    MsgBroadcast(const std::string&,int,int value): frame(value) {}
    Message toMessage() { return {frame}; }
};
int currentPid() { return 0; }
// Observe the actual callback entering its wait; no timing sleeps in the test.
struct ObservedCondition {
    std::condition_variable_any condition;
    std::promise<void> entered;
    template<class L, class D> void wait_for(L& lock,D duration) {
        entered.set_value(); condition.wait_for(lock,duration);
    }
    template<class L, class D, class P> void wait_for(L& lock,D duration,P predicate) {
        entered.set_value(); condition.wait_for(lock,duration,predicate);
    }
    void notify_all() { condition.notify_all(); }
};
struct BroadcastSlot {
    std::future<bool> messageFuture;
    std::shared_ptr<std::atomic_bool> cancelled {std::make_shared<std::atomic_bool>(false)};
    int frame = 0;
    ObservedCondition frameAvailable;
    std::mutex frameMutex;
    struct { bool closed=false; void close() { closed=true; } } sharedMemory;
    bool available = false, run = true;
};
struct IpcBridgePrivate {
    std::map<std::string,BroadcastSlot> m_broadcasts;
    std::mutex m_broadcastsMutex;
    bool frameRequired(const std::string&,Message&);
};
struct IpcBridge {
    IpcBridgePrivate* d;
    void deviceStop(const std::string&);
};
}
'@
$checks = @'
int main() {
    using namespace std::chrono_literals;
    int failure = 0;
    for (bool stopping: {false,true}) {
        AkVCam::IpcBridgePrivate owner;
        auto& slot=owner.m_broadcasts.try_emplace("test").first->second;
        AkVCam::Message result;
        auto entered=slot.frameAvailable.entered.get_future();
        std::promise<void> callbackReturned, allowExit;
        auto callbackReturnedFuture=callbackReturned.get_future();
        auto allowExitFuture=allowExit.get_future();
        slot.messageFuture=std::async(std::launch::async,[&]{
            auto running=owner.frameRequired("test",result);
            if(stopping) { callbackReturned.set_value(); allowExitFuture.wait(); }
            return running;
        });
        if(entered.wait_for(2s)!=std::future_status::ready) return 2;
        AkVCam::IpcBridge bridge{&owner};
        auto started=std::chrono::steady_clock::now();
        auto action=std::async(std::launch::async,[&] {
            if(stopping) bridge.deviceStop("test");
            else {
                // Production write takes the same global -> frame lock order.
                std::lock_guard<std::mutex> global(owner.m_broadcastsMutex);
                std::lock_guard<std::mutex> frame(slot.frameMutex);
                slot.frame=42; slot.available=true; slot.frameAvailable.notify_all();
            }
        });
        bool joinedBeforeRemoval=true;
        if(stopping) {
            joinedBeforeRemoval=callbackReturnedFuture.wait_for(500ms)==std::future_status::ready;
            if(joinedBeforeRemoval) {
                std::lock_guard<std::mutex> global(owner.m_broadcastsMutex);
                joinedBeforeRemoval=action.wait_for(0ms)==std::future_status::timeout
                    && owner.m_broadcasts.count("test")==1 && !slot.sharedMemory.closed;
            }
            allowExit.set_value();
        }
        bool timely=action.wait_for(500ms)==std::future_status::ready;
        action.get();
        printf("%s action_ms=%lld\n",stopping?"stop":"write",(long long)std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now()-started).count());
        if(!stopping) {
            bool running=slot.messageFuture.get();
            if(!timely || !running || result.frame!=42) {
                fprintf(stderr,"FAIL: writer blocked or frame snapshot stale\n"); failure=3;
            }
        } else if(!timely || !joinedBeforeRemoval || !owner.m_broadcasts.empty()) {
            fprintf(stderr,"FAIL: stop did not wake/join before erasing slot\n"); failure=4;
        }
    }
    return failure;
}
'@
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'ipc_frame_test.cpp'),$prefix + "`n" + $callback + $stop + $checks)
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'MSVC Build Tools required.' }
$build = '@echo off' + "`r`n" + 'call "' + (Join-Path $vs 'VC/Auxiliary/Build/vcvars64.bat') + '" >nul' + "`r`n" + 'cl /nologo /EHsc /std:c++17 ipc_frame_test.cpp /Fe:ipc_frame_test.exe'
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'build.cmd'),$build)
Push-Location $OutputDirectory
try {
    & './build.cmd'
    if ($LASTEXITCODE -ne 0) { throw 'IPC regression compilation failed.' }
    & './ipc_frame_test.exe'
    if ($LASTEXITCODE -ne 0) { throw 'IPC frame wakeup regression failed.' }
} finally { Pop-Location }
Write-Output 'PASS: actual frame callback and stop methods promptly wake for write/stop and join before slot removal.'
