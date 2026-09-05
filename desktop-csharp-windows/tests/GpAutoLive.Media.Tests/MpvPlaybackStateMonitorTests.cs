using GpAutoLive.Contracts;
using GpAutoLive.Core;
using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class MpvPlaybackStateMonitorTests
{
    [TestMethod]
    public void SnapshotReadsPlaybackTimeEofAndPausedFromFixedResponses()
    {
        var identity = new MediaPlaybackIdentity(4, 8, 2, 1);
        var created = MpvPlaybackStateMonitor.TryCreateSnapshot(
            identity,
            identity,
            SuccessFrame(1, "12.3456"),
            SuccessFrame(2, "true"),
            SuccessFrame(3, "false"),
            out var snapshot,
            out var error);

        Assert.IsTrue(created);
        Assert.IsNull(error);
        Assert.IsNotNull(snapshot);
        Assert.AreEqual(identity, snapshot.Identity);
        Assert.AreEqual(12_346UL, snapshot.PlaybackTimeMs);
        Assert.IsTrue(snapshot.EofReached);
        Assert.IsFalse(snapshot.Paused);
    }

    [TestMethod]
    public void SnapshotAllowsUnavailableTimeButRejectsInvalidTypedValues()
    {
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        var created = MpvPlaybackStateMonitor.TryCreateSnapshot(
            identity,
            identity,
            SuccessFrame(1, "null"),
            SuccessFrame(2, "false"),
            SuccessFrame(3, "true"),
            out var snapshot,
            out var error);

        Assert.IsTrue(created);
        Assert.IsNull(error);
        Assert.IsNotNull(snapshot);
        Assert.IsNull(snapshot.PlaybackTimeMs);
        Assert.IsFalse(snapshot.EofReached);
        Assert.IsTrue(snapshot.Paused);

        Assert.IsFalse(MpvPlaybackStateMonitor.TryCreateSnapshot(
            identity,
            identity,
            SuccessFrame(1, "-0.1"),
            SuccessFrame(2, "false"),
            SuccessFrame(3, "false"),
            out _,
            out error));
        Assert.AreEqual(MpvPlaybackStateMonitorFailureCode.InvalidPlaybackTime, error?.Code);

        Assert.IsFalse(MpvPlaybackStateMonitor.TryCreateSnapshot(
            identity,
            identity,
            SuccessFrame(1, "1"),
            SuccessFrame(2, "\"yes\""),
            SuccessFrame(3, "false"),
            out _,
            out error));
        Assert.AreEqual(MpvPlaybackStateMonitorFailureCode.InvalidEofValue, error?.Code);
    }

    [TestMethod]
    public void SnapshotRejectsExpiredActiveIdentity()
    {
        var expected = new MediaPlaybackIdentity(5, 9, 1, 0);
        var active = new MediaPlaybackIdentity(6, 9, 1, 0);

        Assert.IsFalse(MpvPlaybackStateMonitor.TryCreateSnapshot(
            expected,
            active,
            SuccessFrame(1, "1"),
            SuccessFrame(2, "false"),
            SuccessFrame(3, "false"),
            out _,
            out var error));
        Assert.AreEqual(MpvPlaybackStateMonitorFailureCode.StalePlaybackIdentity, error?.Code);
    }

    [TestMethod]
    public void SnapshotRejectsMissingResponseFramesWithoutThrowing()
    {
        var identity = new MediaPlaybackIdentity(6, 9, 1, 0);

        Assert.IsFalse(MpvPlaybackStateMonitor.TryCreateSnapshot(
            identity,
            identity,
            playbackTimeFrame: null,
            SuccessFrame(2, "false"),
            SuccessFrame(3, "false"),
            out _,
            out var error));
        Assert.AreEqual(MpvPlaybackStateMonitorFailureCode.InvalidInput, error?.Code);
    }

    [TestMethod]
    public void ApplyingRepeatedSnapshotIsIdempotentAndDifferentIdentityFailsClosed()
    {
        var identity = new MediaPlaybackIdentity(7, 10, 0, 2);
        var candidate = new MpvPlaybackStateSnapshot(identity, 1_000, false, true);

        var first = MpvPlaybackStateMonitor.ApplySnapshot(null, identity, identity, candidate);
        Assert.IsTrue(first.IsSuccess);
        Assert.IsTrue(first.Changed);

        var duplicate = MpvPlaybackStateMonitor.ApplySnapshot(candidate, identity, identity, candidate);
        Assert.IsTrue(duplicate.IsSuccess);
        Assert.IsFalse(duplicate.Changed);

        var stale = MpvPlaybackStateMonitor.ApplySnapshot(
            candidate,
            identity,
            identity,
            candidate with { Identity = new MediaPlaybackIdentity(8, 10, 0, 2) });
        Assert.IsFalse(stale.IsSuccess);
        Assert.AreEqual(MpvPlaybackStateMonitorFailureCode.StalePlaybackIdentity, stale.Error?.Code);
    }

    [TestMethod]
    public async Task PollHonorsCancellationBeforeTouchingIpc()
    {
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(
            $@"\\.\pipe\monitor-cancel-{Guid.NewGuid():N}",
            out var endpoint,
            out var endpointError));
        Assert.IsNull(endpointError);
        Assert.IsNotNull(endpoint);

        await using var gateway = new MpvPlaybackIpcGateway(
            new MpvPlaybackSession(),
            new MpvNamedPipeClient(endpoint!));
        var identity = new MediaPlaybackIdentity(1, 1, 0, 0);
        Assert.IsTrue(gateway.Session.BindSource(CreateVideoSource(identity)).IsSuccess);
        Assert.IsTrue(MpvPlaybackStateMonitor.TryCreate(
            gateway,
            options: null,
            out var monitor,
            out var monitorError));
        Assert.IsNull(monitorError);
        Assert.IsNotNull(monitor);

        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();
        var result = await monitor!.PollOnceAsync(identity, cancellation.Token);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(MpvPlaybackStateMonitorFailureCode.Cancelled, result.Error?.Code);
        Assert.AreEqual(MpvIpcPipeState.Disconnected, gateway.Client.State);
    }

    [TestMethod]
    public async Task MonitorRejectsAnUnboundedPollIntervalAtCreation()
    {
        Assert.IsTrue(MpvIpcPipeEndpoint.TryCreate(
            $@"\\.\pipe\monitor-options-{Guid.NewGuid():N}",
            out var endpoint,
            out var endpointError));
        Assert.IsNull(endpointError);
        Assert.IsNotNull(endpoint);

        await using var gateway = new MpvPlaybackIpcGateway(
            new MpvPlaybackSession(),
            new MpvNamedPipeClient(endpoint!));
        Assert.IsFalse(MpvPlaybackStateMonitor.TryCreate(
            gateway,
            new MpvPlaybackStateMonitorOptions
            {
                PollInterval = TimeSpan.FromMilliseconds(10),
            },
            out var monitor,
            out var monitorError));

        Assert.IsNull(monitor);
        Assert.AreEqual(
            MpvPlaybackStateMonitorFailureCode.InvalidConfiguration,
            monitorError?.Code);
    }

    private static MpvIpcFrame SuccessFrame(ulong requestId, string data) =>
        MpvIpcFrameParser.Parse(
            $"{{\"error\":\"success\",\"request_id\":{requestId},\"data\":{data}}}",
            requestId).Frame!;

    private static MpvActiveSource CreateVideoSource(MediaPlaybackIdentity identity)
    {
        var path = $@"C:\media\monitor-{identity.PlaybackGeneration}.mp4";
        var source = new SourceMediaDto(
            path,
            path,
            MediaKind.Video,
            MediaCompatibilityMode.Direct,
            "monitor.mp4",
            1,
            60_000,
            null,
            null,
            1280,
            720,
            30,
            48_000,
            2,
            "h264",
            "aac",
            null,
            "disabled");
        Assert.IsTrue(MpvActiveSource.TryCreate(source, identity, out var active, out var error));
        Assert.IsNull(error);
        Assert.IsNotNull(active);
        return active;
    }
}
