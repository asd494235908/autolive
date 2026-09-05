using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
public sealed class WindowsPortAudioDeviceEnumeratorTests
{
    [TestMethod]
    public async Task Invalid_path_is_rejected_without_loading_native_library()
    {
        using var enumerator = new WindowsPortAudioDeviceEnumerator();

        var result = await enumerator.ProbeAsync(@"C:\media\portaudio.dll");

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioFailureCode.InvalidPath, result.Error?.Code);
        Assert.IsFalse(result.Snapshot.IsAvailable);
    }

    [TestMethod]
    public async Task Missing_fixed_name_resource_is_reported_as_retryable()
    {
        using var enumerator = new WindowsPortAudioDeviceEnumerator();
        var path = Path.Combine(
            Path.GetTempPath(),
            "gpautolive-portaudio-missing",
            Guid.NewGuid().ToString("N"),
            "portaudio_x64.dll");

        var result = await enumerator.ProbeAsync(path);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioFailureCode.ResourceMissing, result.Error?.Code);
        Assert.IsTrue(result.Error?.Retryable);
    }

    [TestMethod]
    public async Task Malformed_fixed_name_resource_fails_closed()
    {
        var directory = Path.Combine(
            Path.GetTempPath(),
            "gpautolive-portaudio-invalid",
            Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        var path = Path.Combine(directory, "portaudio_x64.dll");
        File.WriteAllBytes(path, [0x00, 0x01, 0x02]);
        try
        {
            using var enumerator = new WindowsPortAudioDeviceEnumerator();

            var result = await enumerator.ProbeAsync(path);

            Assert.IsFalse(result.IsSuccess);
            Assert.AreEqual(WindowsPortAudioFailureCode.NativeLoadFailed, result.Error?.Code);
        }
        finally
        {
            Directory.Delete(directory, recursive: true);
        }
    }

    [TestMethod]
    public async Task Cancellation_before_probe_does_not_load_resource()
    {
        using var enumerator = new WindowsPortAudioDeviceEnumerator();
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();

        var result = await enumerator.ProbeAsync(null, cancellation.Token);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioFailureCode.Cancelled, result.Error?.Code);
    }

    [TestMethod]
    public async Task Dispose_rejects_new_probe_before_path_validation()
    {
        using var enumerator = new WindowsPortAudioDeviceEnumerator();
        enumerator.Dispose();

        var result = await enumerator.ProbeAsync(null);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsPortAudioFailureCode.Closed, result.Error?.Code);
    }
}
