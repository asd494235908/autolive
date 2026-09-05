using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Contracts.Tests;

[TestClass]
public sealed class VirtualCameraContractTests
{
    [TestMethod]
    public void Default_configuration_matches_fixed_yuy2_720p30_contract()
    {
        var config = VirtualCameraConfig.Default;

        Assert.IsTrue(config.TryValidateFixedOutput(out var error), error?.Message);
        Assert.IsTrue(config.TryGetFrameBytes(out error, out var frameBytes), error?.Message);
        Assert.AreEqual("GpAutoLive Camera", config.DeviceName);
        Assert.AreEqual(VirtualCameraPixelFormat.Yuy2, config.PixelFormat);
        Assert.AreEqual(1280U, config.Width);
        Assert.AreEqual(720U, config.Height);
        Assert.AreEqual(30U, config.Fps);
        Assert.IsFalse(config.ZeroCopy);
        Assert.AreEqual(1280 * 720 * 2, frameBytes);
    }

    [TestMethod]
    public void Fixed_configuration_rejects_resolution_or_zero_copy_changes()
    {
        Assert.IsFalse(
            (VirtualCameraConfig.Default with { Width = 1920 }).TryValidateFixedOutput(out var resolutionError));
        StringAssert.Contains(resolutionError!.Message, "YUY2");

        Assert.IsFalse(
            (VirtualCameraConfig.Default with { ZeroCopy = true }).TryValidateFixedOutput(out var transportError));
        StringAssert.Contains(transportError!.Message, "zero_copy=false");
    }

    [TestMethod]
    public void Gpu_facts_reject_warp_and_cpu_conversion()
    {
        var facts = TestFacts() with { IsWarp = true };
        Assert.IsFalse(facts.TryValidateFor(VirtualCameraConfig.Default, out var warpError));
        Assert.AreEqual("virtual_camera_gpu_gate_failed", warpError!.Code);

        facts = TestFacts() with { GpuColorConvert = false };
        Assert.IsFalse(facts.TryValidateFor(VirtualCameraConfig.Default, out var conversionError));
        StringAssert.Contains(conversionError!.Message, "GPU");
    }

    [TestMethod]
    public void Json_contract_keeps_yuy2_literal_and_snake_case_fields()
    {
        var json = JsonSerializer.Serialize(VirtualCameraConfig.Default, ContractJson.CreateOptions());

        StringAssert.Contains(json, "\"pixel_format\":\"YUY2\"");
        StringAssert.Contains(json, "\"zero_copy\":false");
        Assert.IsFalse(json.Contains("DeviceName", StringComparison.Ordinal));
    }

    [TestMethod]
    public void Black_frame_uses_yuv_limited_range_values()
    {
        Assert.IsTrue(
            VirtualCameraFrame.TryCreateBlack(VirtualCameraConfig.Default, 3, 8, 90_000, out var frame, out var error),
            error?.Message);
        Assert.IsNotNull(frame);
        Assert.AreEqual(1280 * 720 * 2, frame!.Payload.Length);
        CollectionAssert.AreEqual(new byte[] { 16, 128, 16, 128 }, frame.Payload[..4]);
    }

    private static GpuCaptureFacts TestFacts() => new(
        VirtualCameraRules.CaptureApi,
        "test-adapter-luid",
        "Test GPU",
        0x1002,
        0x744c,
        "11_0",
        IsWarp: false,
        GpuScale: true,
        GpuColorConvert: true,
        VirtualCameraRules.Transport,
        ZeroCopy: false,
        VirtualCameraRules.Width,
        VirtualCameraRules.Height,
        VirtualCameraRules.Fps);
}
