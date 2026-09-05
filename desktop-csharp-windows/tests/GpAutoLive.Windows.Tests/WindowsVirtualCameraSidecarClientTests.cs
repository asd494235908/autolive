using System.IO.Pipes;
using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsVirtualCameraSidecarClientTests
{
    [TestMethod]
    public async Task Connect_rejects_untrusted_pipe_name_before_opening_a_stream()
    {
        await using var client = new WindowsVirtualCameraSidecarClient();

        var result = await client.ConnectAsync(
            @"\\.\pipe\other",
            TimeSpan.FromSeconds(1));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientErrorCode.InvalidPipeName, result.Error!.Code);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientState.Stopped, result.Snapshot.State);
    }

    [TestMethod]
    public async Task Write_requires_an_active_sidecar_connection()
    {
        await using var client = new WindowsVirtualCameraSidecarClient();
        var frame = CreateFrame(1, 1, 90_000);

        var result = await client.WriteFrameAsync(frame);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientErrorCode.NotConnected, result.Error!.Code);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientState.Stopped, result.Snapshot.State);
    }

    [TestMethod]
    public async Task Connect_and_write_round_trip_fixed_frame_over_local_pipe()
    {
        var pipeName = CreatePipeName();
        var suffix = pipeName[WindowsVirtualCameraSidecarProtocol.PipePrefix.Length..];
        await using var server = new NamedPipeServerStream(
            suffix,
            PipeDirection.In,
            1,
            PipeTransmissionMode.Byte,
            PipeOptions.Asynchronous);
        await using var client = new WindowsVirtualCameraSidecarClient();
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(5));
        var waitTask = server.WaitForConnectionAsync(timeout.Token);

        var connect = await client.ConnectAsync(pipeName, TimeSpan.FromSeconds(2), timeout.Token);
        Assert.IsTrue(connect.IsSuccess, connect.Error?.Message);
        await waitTask;

        var payload = new byte[WindowsVirtualCameraSidecarProtocol.MaxPayloadBytes];
        payload[0] = 16;
        payload[1] = 128;
        var frame = new VirtualCameraFrame(4, 9, 90_000, payload);
        var encoded = new byte[WindowsVirtualCameraSidecarProtocol.EncodedFrameBytes];
        var readTask = server.ReadExactlyAsync(encoded, timeout.Token);
        var write = await client.WriteFrameAsync(frame, timeout.Token);
        Assert.IsTrue(write.IsSuccess, write.Error?.Message);
        await readTask;
        Assert.IsTrue(
            WindowsVirtualCameraSidecarProtocol.TryDecode(encoded, out var decoded, out _, out var decodeError),
            decodeError?.Message);
        Assert.AreEqual(frame.Generation, decoded!.Generation);
        Assert.AreEqual(frame.Sequence, decoded.Sequence);
        Assert.AreEqual(10_000_000L, decoded.Timestamp100Ns);
        CollectionAssert.AreEqual(frame.Payload, decoded.Payload);
        Assert.AreEqual((ulong)1, client.Snapshot.FramesWritten);
        Assert.AreEqual(WindowsVirtualCameraSidecarProtocol.MaxPayloadBytes, client.Snapshot.LastPayloadBytes);

        var stop = await client.StopAsync(timeout.Token);
        Assert.IsTrue(stop.IsSuccess, stop.Error?.Message);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientState.Stopped, stop.Snapshot.State);
    }

    [TestMethod]
    public async Task Timestamp_overflow_is_rejected_without_connecting_or_allocating_frame_buffer()
    {
        await using var client = new WindowsVirtualCameraSidecarClient();
        var frame = CreateFrame(1, 1, ulong.MaxValue);

        var result = await client.WriteFrameAsync(frame);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientErrorCode.TimestampOverflow, result.Error!.Code);
        Assert.AreEqual((ulong)0, result.Snapshot.FramesWritten);
        Assert.IsNull(result.Snapshot.LastPayloadBytes);
    }

    [TestMethod]
    public async Task Stop_is_idempotent_and_dispose_closes_the_client()
    {
        await using var client = new WindowsVirtualCameraSidecarClient();

        var firstStop = await client.StopAsync();
        var secondStop = await client.StopAsync();
        Assert.IsTrue(firstStop.IsSuccess);
        Assert.IsTrue(secondStop.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientState.Stopped, secondStop.Snapshot.State);

        await client.DisposeAsync();
        var write = await client.WriteFrameAsync(CreateFrame(1, 1, 0));
        Assert.IsFalse(write.IsSuccess);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientErrorCode.Closed, write.Error!.Code);
        Assert.AreEqual(WindowsVirtualCameraSidecarClientState.Closed, write.Snapshot.State);
    }

    private static VirtualCameraFrame CreateFrame(ulong generation, ulong sequence, ulong timestamp90Khz) =>
        new(
            generation,
            sequence,
            timestamp90Khz,
            new byte[WindowsVirtualCameraSidecarProtocol.MaxPayloadBytes]);

    private static string CreatePipeName()
    {
        var token = new byte[16];
        Random.Shared.NextBytes(token);
        return WindowsVirtualCameraSidecarProtocol.TryCreatePipeName(token, out var pipeName, out var error)
            ? pipeName!
            : throw new InvalidOperationException(error?.Message);
    }
}
