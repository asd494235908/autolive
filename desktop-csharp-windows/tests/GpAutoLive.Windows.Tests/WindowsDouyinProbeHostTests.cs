using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsDouyinProbeHostTests
{
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
            if (Directory.Exists(Root))
            {
                Directory.Delete(Root, recursive: true);
            }
        }
    }
}
