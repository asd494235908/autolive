using System.Buffers.Binary;
using System.Collections.Immutable;
using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraSidecarHostTests
{
    [TestMethod]
    public async Task Missing_native_ack_returns_bounded_startup_timeout()
    {
        await using var host = new WindowsVirtualCameraSidecarHost();
        var completion = new TaskCompletionSource<WindowsVirtualCameraSidecarHostErrorCode?>(TaskCreationOptions.RunContinuationsAsynchronously);
        typeof(WindowsVirtualCameraSidecarHost).GetField("_outputReady", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!.SetValue(host, completion);
        var result = await host.WaitForOutputReadyAsync();
        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.StartupTimedOut, result.Error!.Code);
    }

    [TestMethod]
    public async Task Native_ack_only_succeeds_for_running_session()
    {
        await using var host = new WindowsVirtualCameraSidecarHost();
        var completion = new TaskCompletionSource<WindowsVirtualCameraSidecarHostErrorCode?>(TaskCreationOptions.RunContinuationsAsynchronously);
        typeof(WindowsVirtualCameraSidecarHost).GetField("_outputReady", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!.SetValue(host, completion);
        completion.SetResult(null);
        Assert.IsFalse((await host.WaitForOutputReadyAsync()).IsSuccess);
        typeof(WindowsVirtualCameraSidecarHost).GetField("_state", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!.SetValue(host, WindowsVirtualCameraSidecarHostState.Running);
        Assert.IsTrue((await host.WaitForOutputReadyAsync()).IsSuccess);
    }

    [TestMethod]
    public async Task Output_confirmation_waits_for_native_ack_and_propagates_fixed_failure()
    {
        await using var host = new WindowsVirtualCameraSidecarHost();
        var completion = new TaskCompletionSource<WindowsVirtualCameraSidecarHostErrorCode?>(TaskCreationOptions.RunContinuationsAsynchronously);
        typeof(WindowsVirtualCameraSidecarHost).GetField("_outputReady", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!.SetValue(host, completion);
        var waiting = host.WaitForOutputReadyAsync();
        Assert.IsFalse(waiting.IsCompleted);
        completion.SetResult(WindowsVirtualCameraSidecarHostErrorCode.ConsumersMustClose);
        var result = await waiting;
        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.ConsumersMustClose, result.Error!.Code);
        StringAssert.Contains(result.Error.Message, "关闭");
    }

    [TestMethod]
    public async Task Output_confirmation_is_cancelled_while_native_ack_is_pending()
    {
        await using var host = new WindowsVirtualCameraSidecarHost();
        var completion = new TaskCompletionSource<WindowsVirtualCameraSidecarHostErrorCode?>(TaskCreationOptions.RunContinuationsAsynchronously);
        typeof(WindowsVirtualCameraSidecarHost).GetField("_outputReady", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!.SetValue(host, completion);
        using var cancellation = new CancellationTokenSource();
        var waiting = host.WaitForOutputReadyAsync(cancellation.Token);
        cancellation.Cancel();
        var result = await waiting;
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.Cancelled, result.Error!.Code);
    }

    [TestMethod]
    public void Native_exit_codes_map_to_fixed_actionable_errors()
    {
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.ConsumersMustClose, WindowsVirtualCameraSidecarHost.MapNativeExitCode(6));
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.OutputFormatRejected, WindowsVirtualCameraSidecarHost.MapNativeExitCode(7));
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.NativeOutputFailed, WindowsVirtualCameraSidecarHost.MapNativeExitCode(5));
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.ProcessExited, WindowsVirtualCameraSidecarHost.MapNativeExitCode(999));
    }

    [TestMethod]
    public async Task Output_confirmation_without_active_session_never_claims_ready()
    {
        await using var host = new WindowsVirtualCameraSidecarHost();
        Assert.IsFalse((await host.WaitForOutputReadyAsync()).IsSuccess);
        using var cancelled = new CancellationTokenSource();
        cancelled.Cancel();
        var result = await host.WaitForOutputReadyAsync(cancelled.Token);
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.Cancelled, result.Error!.Code);
    }

    [TestMethod]
    public async Task Null_plan_is_rejected_before_component_locking()
    {
        await using var host = new WindowsVirtualCameraSidecarHost();
        var result = await host.StartAsync(null);
        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.InvalidPlan, result.Error!.Code);
    }

    [TestMethod]
    public async Task Forged_plan_without_memory_token_is_rejected()
    {
        await using var host = new WindowsVirtualCameraSidecarHost();
        var plan = new WindowsVirtualCameraSidecarLaunchPlan(
            "C:\\invalid\\akvirtualcamera-sidecar-x64.exe",
            "C:\\invalid",
            ImmutableArray.Create("--session-token-stdin"),
            VirtualCameraConfig.Default,
            TimeSpan.FromSeconds(5));

        var result = await host.StartAsync(plan);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.InvalidPlan, result.Error!.Code);
        Assert.AreEqual(WindowsVirtualCameraSidecarHostState.Ready, result.Snapshot.State);
    }

    [TestMethod]
    public async Task Connect_before_start_does_not_expose_pipe_name()
    {
        await using var host = new WindowsVirtualCameraSidecarHost();
        await using var client = new WindowsVirtualCameraSidecarClient();

        var result = await host.ConnectClientAsync(client, TimeSpan.FromSeconds(1));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientErrorCode.InvalidPipeName, result.Error!.Code);
    }

    [TestMethod]
    public async Task Valid_plan_with_non_executable_pe_fails_before_running()
    {
        using var fixture = SidecarFixture.Create();
        var request = new WindowsVirtualCameraSidecarLaunchRequest(
            fixture.Path,
            WindowsVirtualCameraSidecarLaunchPlanBuilder.CreateSessionToken(),
            VirtualCameraConfig.Default,
            TimeSpan.FromSeconds(5));
        Assert.IsTrue(
            WindowsVirtualCameraSidecarLaunchPlanBuilder.TryCreate(request, out var plan, out var planError),
            planError);

        await using var host = new WindowsVirtualCameraSidecarHost();
        var result = await host.StartAsync(plan);

        Assert.IsFalse(result.IsSuccess);
        Assert.IsTrue(
            result.Error!.Code is WindowsVirtualCameraSidecarHostErrorCode.InvalidPlan
                or WindowsVirtualCameraSidecarHostErrorCode.StartFailed
                or WindowsVirtualCameraSidecarHostErrorCode.JobObjectUnavailable);
        Assert.IsFalse(result.Snapshot.State is WindowsVirtualCameraSidecarHostState.Running or WindowsVirtualCameraSidecarHostState.Stopping);
    }

    [TestMethod]
    public async Task Unsigned_sidecar_plan_is_rejected_before_process_creation()
    {
        using var fixture = SidecarFixture.Create();
        var request = new WindowsVirtualCameraSidecarLaunchRequest(
            fixture.Path,
            WindowsVirtualCameraSidecarLaunchPlanBuilder.CreateSessionToken(),
            VirtualCameraConfig.Default,
            TimeSpan.FromSeconds(5));
        Assert.IsTrue(
            WindowsVirtualCameraSidecarLaunchPlanBuilder.TryCreate(request, out var plan, out var planError),
            planError);

        await using var host = new WindowsVirtualCameraSidecarHost();
        var result = await host.StartAsync(plan);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarHostErrorCode.InvalidPlan, result.Error!.Code);
        Assert.AreEqual(WindowsVirtualCameraSidecarHostState.Ready, result.Snapshot.State);
    }

    private sealed class SidecarFixture : IDisposable
    {
        private readonly TemporaryDirectory _directory;

        private SidecarFixture(TemporaryDirectory directory, string path)
        {
            _directory = directory;
            Path = path;
        }

        public string Path { get; }

        public static SidecarFixture Create()
        {
            var directory = new TemporaryDirectory();
            var path = System.IO.Path.Combine(directory.Path, WindowsVirtualCameraSidecarLaunchPlanBuilder.SidecarFileName);
            File.WriteAllBytes(path, CreateMinimalX64Pe());
            return new SidecarFixture(directory, path);
        }

        public void Dispose() => _directory.Dispose();
    }

    private static byte[] CreateMinimalX64Pe()
    {
        var bytes = new byte[512];
        bytes[0] = (byte)'M';
        bytes[1] = (byte)'Z';
        BinaryPrimitives.WriteInt32LittleEndian(bytes.AsSpan(0x3c), 0x80);
        bytes[0x80] = (byte)'P';
        bytes[0x81] = (byte)'E';
        BinaryPrimitives.WriteUInt16LittleEndian(bytes.AsSpan(0x84), 0x8664);
        BinaryPrimitives.WriteUInt16LittleEndian(bytes.AsSpan(0x94), 240);
        BinaryPrimitives.WriteUInt16LittleEndian(bytes.AsSpan(0x98), 0x20b);
        return bytes;
    }

    private sealed class TemporaryDirectory : IDisposable
    {
        public TemporaryDirectory()
        {
            Path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "gpautolive-vc-host-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(Path);
        }

        public string Path { get; }

        public void Dispose()
        {
            if (Directory.Exists(Path))
            {
                Directory.Delete(Path, recursive: true);
            }
        }
    }
}
