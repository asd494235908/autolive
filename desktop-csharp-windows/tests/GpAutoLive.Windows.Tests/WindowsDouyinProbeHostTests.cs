using System.Diagnostics;
using System.Text;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsDouyinProbeHostTests
{
    private static readonly byte[] OnePixelPng =
    [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
        0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
        0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41,
        0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
        0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99,
        0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
        0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82
    ];

    [TestMethod]
    public void Canonical_qr_png_writer_requires_future_valid_png_and_keeps_existing_file()
    {
        using var fixture = ProbeFixture.Create();
        var path = Path.Combine(fixture.Root, "canonical-qr.png");

        Assert.IsTrue(WindowsDouyinProbeHost.TryWriteQrPng(
            path,
            OnePixelPng,
            DateTimeOffset.UtcNow.AddMinutes(5)));
        CollectionAssert.AreEqual(OnePixelPng, File.ReadAllBytes(path));

        Assert.IsFalse(WindowsDouyinProbeHost.TryWriteQrPng(
            path,
            OnePixelPng,
            DateTimeOffset.UtcNow.AddMinutes(5)));
        Assert.IsFalse(WindowsDouyinProbeHost.TryWriteQrPng(
            Path.Combine(fixture.Root, "expired.png"),
            OnePixelPng,
            DateTimeOffset.UtcNow.AddSeconds(-1)));
        Assert.IsFalse(WindowsDouyinProbeHost.TryWriteQrPng(
            Path.Combine(fixture.Root, "not-png.png"),
            [0x01, 0x02],
            DateTimeOffset.UtcNow.AddMinutes(5)));
    }

    [TestMethod]
    public async Task Invalid_plan_fails_before_core_session_starts()
    {
        await using var host = new WindowsDouyinProbeHost(new DouyinLiveManager());

        var result = await host.StartAsync(null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsDouyinProbeHostFailureCode.InvalidPlan, result.Error!.Code);
        Assert.AreEqual(DouyinLiveState.Idle, result.Snapshot.Douyin.State);
        Assert.IsNull(result.Snapshot.ProcessId);
    }

    [TestMethod]
    public async Task Immediately_exiting_probe_is_not_reported_as_success()
    {
        using var fixture = ProbeFixture.Create();
        var manager = new DouyinLiveManager();
        await using var host = new WindowsDouyinProbeHost(manager);
        var completed = new TaskCompletionSource<WindowsDouyinProbeHostSnapshot>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.State is WindowsDouyinProbeHostState.Exited or WindowsDouyinProbeHostState.Failed)
            {
                completed.TrySetResult(snapshot);
            }
        };

        var start = await host.StartAsync(fixture.Request);
        var terminal = start.Snapshot.State is WindowsDouyinProbeHostState.Exited or WindowsDouyinProbeHostState.Failed
            ? start.Snapshot
            : await completed.Task.WaitAsync(TimeSpan.FromSeconds(3));

        Assert.AreNotEqual(DouyinLiveState.Passed, terminal.Douyin.State);
        Assert.IsFalse(terminal.Douyin.Running);
        var stopped = await host.StopAsync();
        Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
        Assert.AreEqual(WindowsDouyinProbeHostState.Stopped, stopped.Snapshot.State);
        Assert.AreEqual(DouyinLiveState.Idle, stopped.Snapshot.Douyin.State);
    }

    [TestMethod]
    public async Task Canonical_sidecar_bootstraps_qr_login_and_binds_live_open_before_listening()
    {
        using var fixture = ProbeFixture.Create();
        var gracefulMarker = Path.Combine(fixture.Root, "graceful-stop.log");
        var manager = new DouyinLiveManager();
        await using var host = new WindowsDouyinProbeHost(
            manager,
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, gracefulMarkerPath: gracefulMarker));
        var listening = new TaskCompletionSource<WindowsDouyinProbeHostSnapshot>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.State == DouyinLiveState.Listening)
            {
                listening.TrySetResult(snapshot);
            }
        };

        var start = await host.StartAsync(fixture.Request with
        {
            Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson
        });
        var snapshot = await listening.Task.WaitAsync(TimeSpan.FromSeconds(5));

        Assert.IsTrue(start.IsSuccess, start.Error?.Message);
        Assert.AreEqual(WindowsDouyinProbeHostState.Running, snapshot.State);
        Assert.AreEqual(DouyinLiveState.Listening, snapshot.Douyin.State);
        Assert.IsTrue(snapshot.Douyin.RoomResolved);
        Assert.IsTrue(snapshot.QrPath is not null && File.Exists(snapshot.QrPath));
        Assert.IsTrue(snapshot.Douyin.Generation > 1);

        var stopped = await host.StopAsync();
        Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
        Assert.AreEqual(WindowsDouyinProbeHostState.Stopped, stopped.Snapshot.State);
        CollectionAssert.AreEqual(
            new[] { "live.close", "auth.logout", "shutdown" },
            File.ReadAllLines(gracefulMarker));
    }

    [TestMethod]
    public async Task Canonical_sidecar_chat_event_is_queued_sent_and_counted_as_accepted()
    {
        using var fixture = ProbeFixture.Create();
        var manager = new DouyinLiveManager();
        await using var host = new WindowsDouyinProbeHost(
            manager,
            _ => CreateCanonicalSidecarStartInfo(fixture.Root, emitChat: true, emitGap: true));
        var accepted = new TaskCompletionSource<WindowsDouyinProbeHostSnapshot>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        host.SnapshotChanged += (_, snapshot) =>
        {
            if (snapshot.Douyin.Metrics.Accepted > 0)
            {
                accepted.TrySetResult(snapshot);
            }
        };

        var start = await host.StartAsync(fixture.Request with
        {
            Protocol = WindowsDouyinProbeProtocol.CanonicalNdjson
        });
        WindowsDouyinProbeHostSnapshot snapshot;
        try
        {
            snapshot = await accepted.Task.WaitAsync(TimeSpan.FromSeconds(5));
        }
        catch (TimeoutException)
        {
            var current = host.Snapshot;
            Assert.Fail($"未观察到 accepted；host={current.State} douyin={current.Douyin.State} event={current.LastEvent} invalid={current.InvalidEventCount} metrics={current.Douyin.Metrics}");
            throw;
        }

        Assert.IsTrue(start.IsSuccess, start.Error?.Message);
        Assert.AreEqual(DouyinLiveState.Listening, snapshot.Douyin.State);
        Assert.AreEqual(1UL, snapshot.Douyin.Metrics.Enqueued);
        Assert.AreEqual(1UL, snapshot.Douyin.Metrics.Dequeued);
        Assert.AreEqual(1UL, snapshot.Douyin.Metrics.Accepted);
        Assert.AreEqual(1UL, snapshot.Douyin.Metrics.GapEvents);
        Assert.AreEqual(2UL, snapshot.Douyin.Metrics.GapDroppedCount);
        Assert.AreEqual("reconnect", snapshot.Douyin.LastGapReason);
        Assert.IsTrue(snapshot.Douyin.ReplyAttempted);

        var stopped = await host.StopAsync();
        Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
    }

    private static ProcessStartInfo CreateCanonicalSidecarStartInfo(
        string workingDirectory,
        bool emitChat = false,
        bool emitGap = false,
        string? gracefulMarkerPath = null)
    {
        var powershell = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.Windows),
            "System32",
            "WindowsPowerShell",
            "v1.0",
            "powershell.exe");
        if (!File.Exists(powershell))
        {
            throw new AssertFailedException("测试环境缺少 Windows PowerShell，无法执行 canonical sidecar 管道测试。");
        }

        const string script = """
            $line = [Console]::In.ReadLine()
            if ($null -eq $line) { exit 2 }
            $qr = $line | ConvertFrom-Json
            if ($qr.op -ne 'auth.qr.start') { exit 3 }
            $qrId = [string]$qr.id
            [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $qrId + '","ok":true,"result":{}}')
            [Console]::Out.WriteLine('{"v":1,"type":"event","event":"auth.qr","payload":{"png_base64":"iVBORw0KGgo=","expires_at_unix_ms":' + ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() + 60000) + '}}')
            [Console]::Out.WriteLine('{"v":1,"type":"event","event":"auth.state","payload":{"state":"confirmed"}}')
            [Console]::Out.Flush()

            $line = [Console]::In.ReadLine()
            if ($null -eq $line) { exit 4 }
            $open = $line | ConvertFrom-Json
            if ($open.op -ne 'live.open') { exit 5 }
            $openId = [string]$open.id
            $generation = [string]$open.payload.generation
            [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $openId + '","ok":true,"result":{"session_id":"ls-' + $generation + '","title":"测试直播","live_status":"connected"}}')
            [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.state","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"state":"connected"}}')
            [Console]::Out.Flush()
            if ($env:GPAUTOLIVE_FAKE_GAP -eq '1') {
                [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.gap","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"reason":"reconnect","dropped_count":2}}')
                [Console]::Out.Flush()
            }
            if ($env:GPAUTOLIVE_FAKE_CHAT -eq '1') {
                [Console]::Out.WriteLine('{"v":1,"type":"event","event":"live.chat","session_id":"ls-' + $generation + '","generation":' + $generation + ',"payload":{"msg_id":"msg-1","received_at_unix_ms":0,"author_id":"author-1","nickname":"观众","content":"hello"}}')
                [Console]::Out.Flush()
                $line = [Console]::In.ReadLine()
                if ($null -eq $line) { exit 6 }
                $send = $line | ConvertFrom-Json
                if ($send.op -ne 'chat.send') { exit 7 }
                $sendId = [string]$send.id
                $actionId = [string]$send.payload.client_action_id
                [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $sendId + '","ok":true,"result":{"state":"accepted","client_action_id":"' + $actionId + '"}}')
                [Console]::Out.Flush()
            }
            while ($null -ne ($line = [Console]::In.ReadLine())) {
                $command = $line | ConvertFrom-Json
                if ($command.op -eq 'live.close') {
                    if ($env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER) { Add-Content -LiteralPath $env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER -Value 'live.close' }
                    $closeId = [string]$command.id
                    [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $closeId + '","ok":true,"result":{}}')
                    [Console]::Out.Flush()
                }
                elseif ($command.op -eq 'shutdown') {
                    if ($env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER) { Add-Content -LiteralPath $env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER -Value 'shutdown' }
                    $shutdownId = [string]$command.id
                    [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $shutdownId + '","ok":true,"result":{}}')
                    [Console]::Out.Flush()
                    exit 0
                }
                elseif ($command.op -eq 'auth.logout') {
                    if ($env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER) { Add-Content -LiteralPath $env:GPAUTOLIVE_FAKE_GRACEFUL_MARKER -Value 'auth.logout' }
                    $logoutId = [string]$command.id
                    [Console]::Out.WriteLine('{"v":1,"type":"response","request_id":"' + $logoutId + '","ok":true,"result":{}}')
                    [Console]::Out.Flush()
                }
            }
            """;
        var startInfo = new ProcessStartInfo
        {
            FileName = powershell,
            WorkingDirectory = workingDirectory,
            UseShellExecute = false,
            CreateNoWindow = true,
            WindowStyle = ProcessWindowStyle.Hidden,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            RedirectStandardInput = true,
            StandardOutputEncoding = new UTF8Encoding(false),
            StandardErrorEncoding = new UTF8Encoding(false),
            StandardInputEncoding = new UTF8Encoding(false)
        };
        startInfo.ArgumentList.Add("-NoProfile");
        startInfo.ArgumentList.Add("-NonInteractive");
        startInfo.ArgumentList.Add("-Command");
        startInfo.ArgumentList.Add(script);
        if (emitChat)
        {
            startInfo.Environment["GPAUTOLIVE_FAKE_CHAT"] = "1";
        }
        if (emitGap)
        {
            startInfo.Environment["GPAUTOLIVE_FAKE_GAP"] = "1";
        }
        if (!string.IsNullOrWhiteSpace(gracefulMarkerPath))
        {
            startInfo.Environment["GPAUTOLIVE_FAKE_GRACEFUL_MARKER"] = gracefulMarkerPath;
        }

        return startInfo;
    }

    private sealed class ProbeFixture : IDisposable
    {
        private ProbeFixture(string root, string script, string qr)
        {
            Root = root;
            Request = new WindowsDouyinProbeLaunchRequest(
                Root,
                script,
                Environment.ProcessPath!,
                "gpautolive-douyin",
                qr,
                new DouyinLiveConfig
                {
                    Enabled = true,
                    RoomId = "12345",
                    Replies = ["收到"],
                    QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
                },
                TimeSpan.FromSeconds(30));
        }

        public string Root { get; }
        public WindowsDouyinProbeLaunchRequest Request { get; }

        public static ProbeFixture Create()
        {
            var root = Path.Combine(Path.GetTempPath(), "gpautolive-douyin-host-tests", Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(Path.Combine(root, "builder"));
            Directory.CreateDirectory(Path.Combine(root, "dy_live"));
            Directory.CreateDirectory(Path.Combine(root, "static"));
            File.WriteAllText(Path.Combine(root, "builder", "auth.py"), "# fixture");
            File.WriteAllText(Path.Combine(root, "dy_live", "server.py"), "# fixture");
            File.WriteAllText(Path.Combine(root, "static", "Live_pb2.py"), "# fixture");
            var script = Path.Combine(root, "probe.py");
            File.WriteAllText(script, "# fixture");
            var qr = Path.Combine(root, "probe.png");
            return new(root, script, qr);
        }

        public void Dispose()
        {
            for (var attempt = 0; attempt < 10 && Directory.Exists(Root); attempt++)
            {
                try
                {
                    Directory.Delete(Root, recursive: true);
                }
                catch (IOException) when (attempt < 9)
                {
                    Thread.Sleep(TimeSpan.FromMilliseconds(50));
                }
                catch (UnauthorizedAccessException) when (attempt < 9)
                {
                    Thread.Sleep(TimeSpan.FromMilliseconds(50));
                }
            }
        }
    }
}
