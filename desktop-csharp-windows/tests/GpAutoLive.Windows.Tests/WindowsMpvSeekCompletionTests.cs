using System.IO.Pipes;
using System.Reflection;
using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsMpvSeekCompletionTests
{
    [TestMethod]
    [DataRow("complete")]
    [DataRow("cancel")]
    [DataRow("timeout")]
    [DataRow("eof")]
    public async Task Seek_requires_a_new_restart_and_finished_seeking(string outcome)
    {
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(@"\\.\pipe\autolive-seek-" + Guid.NewGuid().ToString("N"),
            out var endpoint, out _));
        await using var server = new NamedPipeServerStream(endpoint!.PipeName, PipeDirection.InOut, 1,
            PipeTransmissionMode.Byte, PipeOptions.Asynchronous);
        await using var client = new MpvNamedPipeClient(endpoint);
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        const string path = @"C:\media\seek.mp4";
        var source = new SourceMediaDto(path, path, MediaKind.Video, MediaCompatibilityMode.Direct,
            "seek.mp4", 1, 60_000, null, null, 1280, 720, 30, 48000, 2, "h264", "aac", null, "disabled");
        Assert.IsTrue(MpvActiveSource.TryCreate(source, identity, out var active, out _));
        var session = new MpvPlaybackSession();
        Assert.IsTrue(session.BindSource(active).IsSuccess);
        Assert.IsTrue(session.Start().IsSuccess);
        await using var gateway = new MpvPlaybackIpcGateway(session, client);
        await using var runtime = new WindowsMpvPlaybackRuntime();
        await using var controller = new WindowsMpvPlaybackController();
        var host = (WindowsMpvProcessHost)GetField(runtime, "_host")!;
        SetField(host, "_state", WindowsMpvHostState.Running);
        SetField(runtime, "_state", WindowsMpvPlaybackRuntimeState.Running);
        SetField(runtime, "_gateway", gateway);
        SetField(runtime, "_session", session);
        Assert.IsTrue(MpvPlaybackStateMonitor.TryCreate(gateway, null, out var stateMonitor, out _));
        SetField(runtime, "_stateMonitor", stateMonitor);
        SetField(controller, "_runtime", runtime);
        SetField(controller, "_session", session);
        SetField(controller, "_activeIdentity", identity);
        SetField(controller, "_state", WindowsMpvPlaybackControllerState.Playing);

        var oldFalseObserved = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var newRestartObserved = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var releaseRestart = 0;
        var releaseSeeking = 0;
        var serverTask = Task.Run(async () =>
        {
            await server.WaitForConnectionAsync();
            using var reader = new StreamReader(server, leaveOpen: true);
            using var writer = new StreamWriter(server, leaveOpen: true) { AutoFlush = true };
            var seekReceived = false;
            var oldRestartSent = false;
            var newRestartSent = false;
            while (await reader.ReadLineAsync() is { } line)
            {
                using var request = JsonDocument.Parse(line);
                var command = request.RootElement.GetProperty("command");
                var operation = command[0].GetString();
                if (!oldRestartSent)
                {
                    await writer.WriteLineAsync("{\"event\":\"file-loaded\"}");
                    await writer.WriteLineAsync("{\"event\":\"playback-restart\"}");
                    oldRestartSent = true;
                }
                if (operation == "seek") seekReceived = true;
                if (seekReceived && Volatile.Read(ref releaseRestart) != 0 && !newRestartSent)
                {
                    await writer.WriteLineAsync("{\"event\":\"playback-restart\"}");
                    newRestartSent = true;
                }
                object? data = operation == "get_property" ? command[1].GetString() switch
                {
                    "seeking" => newRestartSent && Volatile.Read(ref releaseSeeking) == 0,
                    "pause" => false,
                    "eof-reached" => outcome == "eof" && seekReceived && Volatile.Read(ref releaseSeeking) != 0,
                    "time-pos" => 18.0,
                    "path" => path,
                    _ => null,
                } : null;
                await writer.WriteLineAsync(JsonSerializer.Serialize(new
                {
                    request_id = request.RootElement.GetProperty("request_id").GetUInt64(),
                    error = "success",
                    data,
                }));
                if (seekReceived && operation == "get_property")
                {
                    if (newRestartSent) newRestartObserved.TrySetResult();
                    else oldFalseObserved.TrySetResult();
                }
            }
        });
        using var cancellation = new CancellationTokenSource();
        Task<WindowsMpvPlaybackControllerResult>? seek = null;
        try
        {
            Assert.IsTrue((await gateway.ConnectAsync()).IsSuccess);
            seek = controller.SeekAsync(identity, outcome == "eof" ? 60_000UL : 18_000UL, cancellation.Token);
            await Task.WhenAny(seek, oldFalseObserved.Task).WaitAsync(TimeSpan.FromSeconds(3));
            Assert.IsFalse(seek.IsCompleted, "ACK 和旧 seeking=false 不能代表本次 seek 完成。");
            if (outcome == "eof")
            {
                Volatile.Write(ref releaseSeeking, 1);
                Assert.IsTrue((await seek.WaitAsync(TimeSpan.FromSeconds(3))).IsSuccess,
                    "已知末端目标到达真实 EOF 时不要求不存在的 playback-restart。");
                return;
            }
            Volatile.Write(ref releaseRestart, 1);
            // 真实监视器共用原有 IPC 串行门；无论哪一条读取消费事件，都保留同一序列。
            var monitor = runtime.PollPlaybackStateAsync(identity);
            await newRestartObserved.Task.WaitAsync(TimeSpan.FromSeconds(3));
            Assert.IsFalse(seek.IsCompleted, "新 restart 到达但 seeking 仍为 true 时不能报告完成。");
            if (outcome == "complete") Volatile.Write(ref releaseSeeking, 1);
            if (outcome == "cancel") cancellation.Cancel();
            var result = await seek.WaitAsync(TimeSpan.FromSeconds(4));
            Assert.AreEqual(outcome == "complete", result.IsSuccess);
            if (outcome == "cancel")
                Assert.AreEqual(WindowsMpvPlaybackControllerFailureCode.Cancelled, result.Error?.Code);
            if (outcome == "timeout")
                Assert.AreEqual(WindowsMpvPlaybackControllerFailureCode.EffectiveFrameNotObserved, result.Error?.Code);
            Assert.IsTrue((await monitor.WaitAsync(TimeSpan.FromSeconds(3))).IsSuccess);
        }
        finally
        {
            cancellation.Cancel();
            if (seek is not null) await seek.WaitAsync(TimeSpan.FromSeconds(4));
            SetField(controller, "_runtime", null);
            SetField(controller, "_session", null);
            SetField(runtime, "_gateway", null);
            SetField(runtime, "_session", null);
            SetField(runtime, "_stateMonitor", null);
            SetField(host, "_state", WindowsMpvHostState.Ready);
            await gateway.DisposeAsync();
            await serverTask.WaitAsync(TimeSpan.FromSeconds(3));
        }
    }

    private static object? GetField(object owner, string name) => owner.GetType()
        .GetField(name, BindingFlags.NonPublic | BindingFlags.Instance)!.GetValue(owner);

    private static void SetField(object owner, string name, object? value) => owner.GetType()
        .GetField(name, BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(owner, value);
}
