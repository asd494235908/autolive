using System.IO.Pipes;
using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraSidecarOutputWriterTests
{
    [TestMethod]
    public async Task Writer_sends_a_fixed_black_frame_when_output_policy_is_black()
    {
        var manager = ReadyManager();
        var pipeName = CreatePipeName();
        await using var server = CreateServer(pipeName);
        await using var client = new WindowsVirtualCameraSidecarClient();
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(5));
        var waitForConnection = server.WaitForConnectionAsync(timeout.Token);

        var connected = await client.ConnectAsync(pipeName, TimeSpan.FromSeconds(2), timeout.Token);
        Assert.IsTrue(connected.IsSuccess, connected.Error?.Message);
        await waitForConnection;

        await using var writer = new WindowsVirtualCameraSidecarOutputWriter(
            manager,
            client,
            TimeSpan.FromTicks(TimeSpan.TicksPerSecond / VirtualCameraRules.Fps));
        var started = await writer.StartAsync(timeout.Token);
        Assert.IsTrue(started.IsSuccess, started.Error?.Message);

        var encoded = new byte[WindowsVirtualCameraSidecarProtocol.EncodedFrameBytes];
        await server.ReadExactlyAsync(encoded, timeout.Token);
        Assert.IsTrue(
            WindowsVirtualCameraSidecarProtocol.TryDecode(encoded, out var decoded, out _, out var error),
            error?.Message);
        Assert.AreEqual((byte)16, decoded!.Payload[0]);
        Assert.AreEqual((byte)128, decoded.Payload[1]);
        Assert.AreEqual((byte)16, decoded.Payload[2]);
        Assert.AreEqual((byte)128, decoded.Payload[3]);
        Assert.AreEqual(1UL, writer.Snapshot.FramesWritten);
        Assert.AreEqual(1UL, writer.Snapshot.BlackFramesWritten);
        Assert.AreEqual(0UL, writer.Snapshot.LatestFramesWritten);

        var stopped = await writer.StopAsync(timeout.Token);
        Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
    }

    [TestMethod]
    public async Task Writer_forwards_latest_frame_without_copying_or_replacing_payload()
    {
        var manager = ReadyManager();
        var generation = manager.Snapshot.Generation;
        manager.SetOutputContext(new(true, true, false, false, false, true));
        var payload = new byte[WindowsVirtualCameraSidecarProtocol.MaxPayloadBytes];
        payload[0] = 90;
        payload[1] = 100;
        payload[2] = 110;
        payload[3] = 120;
        Assert.IsTrue(manager.SubmitFrame(new VirtualCameraFrame(generation, 77, 90_000, payload)).IsSuccess);

        var pipeName = CreatePipeName();
        await using var server = CreateServer(pipeName);
        await using var client = new WindowsVirtualCameraSidecarClient();
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(5));
        var waitForConnection = server.WaitForConnectionAsync(timeout.Token);
        var connected = await client.ConnectAsync(pipeName, TimeSpan.FromSeconds(2), timeout.Token);
        Assert.IsTrue(connected.IsSuccess, connected.Error?.Message);
        await waitForConnection;

        await using var writer = new WindowsVirtualCameraSidecarOutputWriter(
            manager,
            client,
            TimeSpan.FromTicks(TimeSpan.TicksPerSecond / VirtualCameraRules.Fps));
        var started = await writer.StartAsync(timeout.Token);
        Assert.IsTrue(started.IsSuccess, started.Error?.Message);

        var encoded = new byte[WindowsVirtualCameraSidecarProtocol.EncodedFrameBytes];
        await server.ReadExactlyAsync(encoded, timeout.Token);
        Assert.IsTrue(
            WindowsVirtualCameraSidecarProtocol.TryDecode(encoded, out var decoded, out _, out var error),
            error?.Message);
        Assert.AreEqual(generation, decoded!.Generation);
        Assert.AreEqual(10_000_000L, decoded.Timestamp100Ns);
        Assert.AreEqual((byte)90, decoded.Payload[0]);
        Assert.AreEqual((byte)100, decoded.Payload[1]);
        Assert.AreEqual(1UL, writer.Snapshot.FramesWritten);
        Assert.AreEqual(0UL, writer.Snapshot.BlackFramesWritten);
        Assert.AreEqual(1UL, writer.Snapshot.LatestFramesWritten);

        var stopped = await writer.StopAsync(timeout.Token);
        Assert.IsTrue(stopped.IsSuccess, stopped.Error?.Message);
    }

    [TestMethod]
    public async Task Writer_requires_a_connected_client_and_stop_is_idempotent()
    {
        await using var client = new WindowsVirtualCameraSidecarClient();
        await using var writer = new WindowsVirtualCameraSidecarOutputWriter(ReadyManager(), client);

        var start = await writer.StartAsync();
        Assert.IsFalse(start.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarOutputWriterErrorCode.NotConnected, start.Error!.Code);

        var firstStop = await writer.StopAsync();
        var secondStop = await writer.StopAsync();
        Assert.IsTrue(firstStop.IsSuccess);
        Assert.IsTrue(secondStop.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarOutputWriterState.Stopped, secondStop.Snapshot.State);
    }

    [TestMethod]
    public async Task Coordinator_requires_install_gate_before_sidecar_plan()
    {
        await using var coordinator = new WindowsVirtualCameraOutputCoordinator(
            new VirtualCameraOutputManager(),
            new WindowsVirtualCameraSurfaceBinding());

        var result = await coordinator.StartAsync(null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraOutputCoordinatorCode.InvalidState, result.Code);
    }

    [TestMethod]
    public async Task Coordinator_rejects_missing_plan_after_install_gate()
    {
        var manager = new VirtualCameraOutputManager();
        Assert.IsTrue(manager.MarkInstalled().IsSuccess);
        await using var coordinator = new WindowsVirtualCameraOutputCoordinator(
            manager,
            new WindowsVirtualCameraSurfaceBinding());

        var result = await coordinator.StartAsync(null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraOutputCoordinatorCode.InvalidPlan, result.Code);
    }

    private static NamedPipeServerStream CreateServer(string pipeName) => new(
        pipeName[WindowsVirtualCameraSidecarProtocol.PipePrefix.Length..],
        PipeDirection.In,
        1,
        PipeTransmissionMode.Byte,
        PipeOptions.Asynchronous);

    private static string CreatePipeName()
    {
        var token = WindowsVirtualCameraSidecarLaunchPlanBuilder.CreateSessionToken();
        return WindowsVirtualCameraSidecarProtocol.TryCreatePipeName(token, out var pipeName, out var error)
            ? pipeName!
            : throw new InvalidOperationException(error?.Message);
    }

    private static VirtualCameraOutputManager ReadyManager()
    {
        var manager = new VirtualCameraOutputManager();
        Assert.IsTrue(manager.MarkInstalled().IsSuccess);
        Assert.IsTrue(manager.BeginStart().IsSuccess);
        Assert.IsTrue(manager.MarkReady(new GpuCaptureFacts(
            VirtualCameraRules.CaptureApi,
            "test-adapter",
            "Test GPU",
            0x1002,
            0x744c,
            "11_0",
            false,
            true,
            true,
            VirtualCameraRules.Transport,
            false,
            VirtualCameraRules.Width,
            VirtualCameraRules.Height,
            VirtualCameraRules.Fps)).IsSuccess);
        return manager;
    }
}
