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
        var encoded = new byte[WindowsVirtualCameraSidecarProtocol.EncodedFrameBytes];
        var readFrame = server.ReadExactlyAsync(encoded, timeout.Token);
        var started = await writer.StartAsync(timeout.Token);
        Assert.IsTrue(started.IsSuccess, started.Error?.Message);
        await readFrame;
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
        var encoded = new byte[WindowsVirtualCameraSidecarProtocol.EncodedFrameBytes];
        var readFrame = server.ReadExactlyAsync(encoded, timeout.Token);
        var started = await writer.StartAsync(timeout.Token);
        Assert.IsTrue(started.IsSuccess, started.Error?.Message);
        await readFrame;
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
    public async Task Writer_start_does_not_succeed_when_first_frame_write_fails()
    {
        var pipeName = CreatePipeName();
        await using var server = CreateServer(pipeName);
        await using var client = new WindowsVirtualCameraSidecarClient();
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(5));
        var waitForConnection = server.WaitForConnectionAsync(timeout.Token);
        var connected = await client.ConnectAsync(pipeName, TimeSpan.FromSeconds(2), timeout.Token);
        Assert.IsTrue(connected.IsSuccess, connected.Error?.Message);
        await waitForConnection;
        await server.DisposeAsync();

        await using var writer = new WindowsVirtualCameraSidecarOutputWriter(ReadyManager(), client);
        var started = await writer.StartAsync(timeout.Token);

        Assert.IsFalse(started.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarOutputWriterErrorCode.WriteFailed, started.Error!.Code);
        Assert.AreEqual(WindowsVirtualCameraSidecarOutputWriterState.Failed, started.Snapshot.State);
    }

    [TestMethod]
    public async Task Coordinator_stop_does_not_report_success_when_writer_stop_is_cancelled()
    {
        var manager = ReadyManager();
        await using var client = new WindowsVirtualCameraSidecarClient();
        await using var writer = new WindowsVirtualCameraSidecarOutputWriter(manager, client);
        using var stopCancellation = new CancellationTokenSource();
        using var workerCancellation = new CancellationTokenSource();
        var workerCompleted = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        using var cancellationRegistration = workerCancellation.Token.Register(() =>
        {
            stopCancellation.Cancel();
            workerCompleted.TrySetResult(true);
        });
        var writerType = typeof(WindowsVirtualCameraSidecarOutputWriter);
        writerType.GetField("_worker", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!
            .SetValue(writer, workerCompleted.Task);
        writerType.GetField("_cancellation", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!
            .SetValue(writer, workerCancellation);
        writerType.GetField("_state", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!
            .SetValue(writer, WindowsVirtualCameraSidecarOutputWriterState.Running);

        await using var coordinator = new WindowsVirtualCameraOutputCoordinator(
            manager,
            new WindowsVirtualCameraSurfaceBinding());
        typeof(WindowsVirtualCameraOutputCoordinator)
            .GetField("_writer", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!
            .SetValue(coordinator, writer);

        var stopTask = coordinator.StopAsync(stopCancellation.Token);
        var stopped = await stopTask;

        Assert.IsFalse(stopped.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraOutputCoordinatorCode.CleanupFailed, stopped.Code);
        Assert.AreEqual(VirtualCameraState.Installed, stopped.Snapshot.Output.State);
        Assert.AreEqual(WindowsVirtualCameraSidecarOutputWriterState.Closed, stopped.Snapshot.Writer.State);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientState.Stopped, stopped.Snapshot.Client.State);
        Assert.AreEqual(WindowsVirtualCameraSidecarHostState.Stopped, stopped.Snapshot.Sidecar.State);

        var blockedStart = await coordinator.StartAsync(null);
        Assert.IsFalse(blockedStart.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraOutputCoordinatorCode.CleanupFailed, blockedStart.Code);

        var repeatedStop = await coordinator.StopAsync();
        Assert.IsTrue(repeatedStop.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraOutputCoordinatorCode.Stopped, repeatedStop.Code);
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

    [TestMethod]
    public async Task Coordinator_reports_runtime_component_failure_and_keeps_stop_cleanup_available()
    {
        var manager = new VirtualCameraOutputManager();
        Assert.IsTrue(manager.MarkInstalled().IsSuccess);
        await using var coordinator = new WindowsVirtualCameraOutputCoordinator(
            manager,
            new WindowsVirtualCameraSurfaceBinding());

        var host = typeof(WindowsVirtualCameraOutputCoordinator)
            .GetField("_sidecarHost", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!
            .GetValue(coordinator)!;
        host.GetType()
            .GetField("_state", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!
            .SetValue(host, WindowsVirtualCameraSidecarHostState.Failed);
        typeof(WindowsVirtualCameraOutputCoordinator)
            .GetField("_started", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!
            .SetValue(coordinator, true);

        WindowsVirtualCameraOutputCoordinatorSnapshot? changed = null;
        coordinator.SnapshotChanged += (_, snapshot) => changed = snapshot;

        coordinator.RefreshHealth();

        Assert.AreEqual(VirtualCameraState.Failed, manager.Snapshot.State);
        Assert.IsTrue(coordinator.HasActiveResources);
        Assert.AreEqual(VirtualCameraState.Failed, changed?.Output.State);

        var stopped = await coordinator.StopAsync();

        Assert.IsTrue(stopped.IsSuccess, stopped.ErrorMessage);
        Assert.AreEqual(WindowsVirtualCameraOutputCoordinatorCode.Stopped, stopped.Code);
        Assert.AreEqual(VirtualCameraState.Installed, stopped.Snapshot.Output.State);
    }

    [TestMethod]
    public async Task Coordinator_publishes_ready_streaming_projection_when_downstream_count_changes()
    {
        var manager = ReadyManager();
        await using var coordinator = new WindowsVirtualCameraOutputCoordinator(
            manager,
            new WindowsVirtualCameraSurfaceBinding());

        var host = typeof(WindowsVirtualCameraOutputCoordinator)
            .GetField("_sidecarHost", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!
            .GetValue(coordinator)!;
        var publishCount = host.GetType()
            .GetMethod("PublishDownstreamClientCount", System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic)!;

        var snapshots = new List<WindowsVirtualCameraOutputCoordinatorSnapshot>();
        coordinator.SnapshotChanged += (_, snapshot) => snapshots.Add(snapshot);

        publishCount.Invoke(host, [2u]);
        publishCount.Invoke(host, [0u]);

        Assert.AreEqual(VirtualCameraState.Ready, manager.Snapshot.State);
        Assert.AreEqual(2, snapshots.Count);
        Assert.AreEqual(VirtualCameraState.Streaming, snapshots[0].Output.State);
        Assert.AreEqual((uint)2, snapshots[0].Output.DownstreamClientCount);
        Assert.AreEqual(VirtualCameraState.Ready, snapshots[1].Output.State);
        Assert.AreEqual((uint)0, snapshots[1].Output.DownstreamClientCount);
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
