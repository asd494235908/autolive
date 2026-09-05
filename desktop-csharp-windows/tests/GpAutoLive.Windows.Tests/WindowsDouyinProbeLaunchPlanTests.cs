using GpAutoLive.Contracts;
using GpAutoLive.Core.Processes;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsDouyinProbeLaunchPlanTests
{
    [TestMethod]
    public void Plan_uses_argument_list_and_reuses_normalized_local_config()
    {
        using var fixture = ProbeFixture.Create();
        var request = fixture.CreateRequest(new DouyinLiveConfig
        {
            Enabled = true,
            RoomId = " https://live.douyin.com/12345 ",
            Replies = [" A ", "A", "B"],
            QueueCapacity = 500,
        });

        var ok = WindowsDouyinProbeLaunchPlanBuilder.TryCreate(request, out var plan, out var error);

        Assert.IsTrue(ok, error);
        Assert.IsNotNull(plan);
        Assert.AreEqual(fixture.CondaPath, plan!.ExecutablePath);
        CollectionAssert.Contains(plan.Arguments.ToArray(), "12345");
        CollectionAssert.AreEqual(
            new[] { "A", "B" },
            plan.Arguments
                .SkipWhile(argument => argument != "--reply")
                .Skip(1)
                .Where((_, index) => index % 2 == 0)
                .Take(2)
                .ToArray());
        Assert.AreEqual(ProcessLaunchPolicy.HiddenNoShellProcessTree, plan.LaunchPolicy);
        Assert.AreEqual(30 * 1_000, plan.Timeout.TotalMilliseconds);
    }

    [TestMethod]
    public void Plan_rejects_missing_upstream_contract_files()
    {
        using var fixture = ProbeFixture.Create();
        File.Delete(Path.Combine(fixture.UpstreamRoot, "dy_live", "server.py"));

        var ok = WindowsDouyinProbeLaunchPlanBuilder.TryCreate(
            fixture.CreateRequest(DouyinLiveConfig.Default),
            out var plan,
            out var error);

        Assert.IsFalse(ok);
        Assert.IsNull(plan);
        StringAssert.Contains(error, "缺少必要文件");
    }

    [TestMethod]
    public void Plan_rejects_unsafe_environment_and_fractional_timeout()
    {
        using var fixture = ProbeFixture.Create();
        var invalidEnvironment = fixture.CreateRequest(DouyinLiveConfig.Default) with { CondaEnvironment = "gp;autolive" };
        var invalidTimeout = fixture.CreateRequest(DouyinLiveConfig.Default) with { Timeout = TimeSpan.FromSeconds(30.5) };

        Assert.IsFalse(WindowsDouyinProbeLaunchPlanBuilder.TryCreate(invalidEnvironment, out _, out var environmentError));
        Assert.IsTrue(environmentError!.Contains("环境名", StringComparison.Ordinal));
        Assert.IsFalse(WindowsDouyinProbeLaunchPlanBuilder.TryCreate(invalidTimeout, out _, out var timeoutError));
        Assert.IsTrue(timeoutError!.Contains("超时", StringComparison.Ordinal));
    }

    [TestMethod]
    public void Plan_rejects_non_png_or_unavailable_output_directory()
    {
        using var fixture = ProbeFixture.Create();
        var invalidPath = fixture.CreateRequest(DouyinLiveConfig.Default) with
        {
            QrOutputPath = Path.Combine(fixture.Root, "qr.txt")
        };

        var ok = WindowsDouyinProbeLaunchPlanBuilder.TryCreate(invalidPath, out var plan, out var error);

        Assert.IsFalse(ok);
        Assert.IsNull(plan);
        StringAssert.Contains(error, "PNG");
    }

    private sealed class ProbeFixture : IDisposable
    {
        private ProbeFixture(string root)
        {
            Root = root;
            UpstreamRoot = Path.Combine(root, "Douyin_Spider");
            CondaPath = Path.Combine(root, "conda.exe");
            ScriptPath = Path.Combine(root, "douyin_live_compat_probe.py");
            QrOutputPath = Path.Combine(root, "qr.png");
            Directory.CreateDirectory(Path.Combine(UpstreamRoot, "builder"));
            Directory.CreateDirectory(Path.Combine(UpstreamRoot, "dy_live"));
            Directory.CreateDirectory(Path.Combine(UpstreamRoot, "static"));
            File.WriteAllText(Path.Combine(UpstreamRoot, "builder", "auth.py"), "# test");
            File.WriteAllText(Path.Combine(UpstreamRoot, "dy_live", "server.py"), "# test");
            File.WriteAllText(Path.Combine(UpstreamRoot, "static", "Live_pb2.py"), "# test");
            File.WriteAllText(CondaPath, "test");
            File.WriteAllText(ScriptPath, "# test");
        }

        public string Root { get; }
        public string UpstreamRoot { get; }
        public string CondaPath { get; }
        public string ScriptPath { get; }
        public string QrOutputPath { get; }

        public static ProbeFixture Create() => new(
            Path.Combine(Path.GetTempPath(), "GpAutoLive.CSharp.Tests", Guid.NewGuid().ToString("N")));

        public WindowsDouyinProbeLaunchRequest CreateRequest(DouyinLiveConfig config) => new(
            UpstreamRoot,
            ScriptPath,
            CondaPath,
            "gpautolive-douyin",
            QrOutputPath,
            config,
            TimeSpan.FromSeconds(30));

        public void Dispose()
        {
            if (Directory.Exists(Root))
            {
                Directory.Delete(Root, recursive: true);
            }
        }
    }
}
