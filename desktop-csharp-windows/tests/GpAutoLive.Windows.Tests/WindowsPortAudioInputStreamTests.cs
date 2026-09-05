using GpAutoLive.Media;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsPortAudioInputStreamTests
{
    [TestMethod]
    public async Task Invalid_path_is_rejected_before_native_load()
    {
        var destination = new AudioPcmRingBuffer(256, 1);
        using var input = new WindowsPortAudioInputStream(destination);

        var result = await input.StartAsync(
            @"C:\media\portaudio.dll",
            new WindowsPortAudioInputConfig(0));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioInputFailureCode.InvalidPath, result.Error?.Code);
        Assert.IsFalse(input.Snapshot.IsRunning);
    }

    [TestMethod]
    public async Task Missing_resource_is_retryable()
    {
        var destination = new AudioPcmRingBuffer(256, 1);
        using var input = new WindowsPortAudioInputStream(destination);
        var path = Path.Combine(
            Path.GetTempPath(),
            "gpautolive-portaudio-input-missing",
            Guid.NewGuid().ToString("N"),
            "portaudio_x64.dll");

        var result = await input.StartAsync(path, new WindowsPortAudioInputConfig(0));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioInputFailureCode.ResourceMissing, result.Error?.Code);
        Assert.IsTrue(result.Error?.Retryable);
    }

    [TestMethod]
    public async Task Invalid_config_is_rejected_without_resource_access()
    {
        var destination = new AudioPcmRingBuffer(256, 1);
        using var input = new WindowsPortAudioInputStream(destination);
        var path = Path.Combine(
            Path.GetTempPath(),
            "gpautolive-portaudio-input-invalid-config",
            Guid.NewGuid().ToString("N"),
            "portaudio_x64.dll");

        var result = await input.StartAsync(
            path,
            new WindowsPortAudioInputConfig(0, Channels: 3));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioInputFailureCode.InvalidConfig, result.Error?.Code);
    }

    [TestMethod]
    public async Task Cancellation_before_start_does_not_load_resource()
    {
        var destination = new AudioPcmRingBuffer(256, 1);
        using var input = new WindowsPortAudioInputStream(destination);
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();

        var result = await input.StartAsync(null, new WindowsPortAudioInputConfig(0), cancellation.Token);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioInputFailureCode.Cancelled, result.Error?.Code);
    }

    [TestMethod]
    public async Task Dispose_rejects_new_start()
    {
        var destination = new AudioPcmRingBuffer(256, 1);
        var input = new WindowsPortAudioInputStream(destination);
        input.Dispose();

        var result = await input.StartAsync(null, new WindowsPortAudioInputConfig(0));

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioInputFailureCode.Closed, result.Error?.Code);
    }

    [TestMethod]
    public void Snapshot_without_stream_exposes_conservative_health_fallback()
    {
        var destination = new AudioPcmRingBuffer(256, 1);
        using var input = new WindowsPortAudioInputStream(destination);

        var snapshot = input.Snapshot;

        Assert.AreEqual(WindowsPortAudioHardwareState.NotCreated, snapshot.HardwareState);
        Assert.AreEqual((ulong)0, snapshot.CallbackCount);
        Assert.AreEqual((uint)0, snapshot.LastCallbackStatusFlags);
    }
}
