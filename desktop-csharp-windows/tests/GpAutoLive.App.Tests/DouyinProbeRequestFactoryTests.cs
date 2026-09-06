using GpAutoLive.App.Features.Douyin;
using GpAutoLive.Contracts;
using GpAutoLive.Windows;

namespace GpAutoLive.App.Tests;

[TestClass]
public sealed class DouyinProbeRequestFactoryTests
{
    [TestMethod]
    public void Empty_environment_keeps_default_local_mode()
    {
        var result = DouyinProbeRequestFactory.TryCreate(
            Config(),
            _ => null,
            "C:\\Temp",
            out var request,
            out var error,
            out var configured);

        Assert.IsFalse(result);
        Assert.IsFalse(configured);
        Assert.IsNull(request);
        Assert.IsNull(error);
    }

    [TestMethod]
    public void Partial_environment_is_rejected_without_creating_request()
    {
        var result = DouyinProbeRequestFactory.TryCreate(
            Config(),
            name => name == "AUTOLIVE_DOUYIN_ROOT" ? "C:\\Douyin_Spider" : null,
            "C:\\Temp",
            out var request,
            out var error,
            out var configured);

        Assert.IsFalse(result);
        Assert.IsTrue(configured);
        Assert.IsNull(request);
        StringAssert.Contains(error!, "必须同时提供");
    }

    [TestMethod]
    public void Complete_environment_creates_bounded_request_and_unique_qr_path()
    {
        var values = new Dictionary<string, string?>(StringComparer.Ordinal)
        {
            ["AUTOLIVE_DOUYIN_ROOT"] = "C:\\Douyin_Spider",
            ["AUTOLIVE_DOUYIN_PROBE"] = "C:\\probe.py",
            ["CONDA_EXE"] = "C:\\Miniconda3\\Scripts\\conda.exe",
            ["AUTOLIVE_CONDA_ENV"] = "gpautolive-douyin-test",
            ["AUTOLIVE_DOUYIN_TIMEOUT_SEC"] = "120",
            ["AUTOLIVE_DOUYIN_PROTOCOL"] = "canonical"
        };

        Assert.IsTrue(
            DouyinProbeRequestFactory.TryCreate(
                Config(),
                name => values[name],
                "C:\\Temp",
                out var request,
                out var error,
                out var configured),
            error);

        Assert.IsTrue(configured);
        Assert.AreEqual("gpautolive-douyin-test", request!.CondaEnvironment);
        Assert.AreEqual(TimeSpan.FromSeconds(120), request.Timeout);
        Assert.AreEqual(WindowsDouyinProbeProtocol.CanonicalNdjson, request.Protocol);
        StringAssert.StartsWith(request.QrOutputPath, "C:\\Temp\\gpautolive-douyin-");
        StringAssert.EndsWith(request.QrOutputPath, ".png");
    }

    [TestMethod]
    public void Invalid_timeout_is_rejected_before_sidecar_plan()
    {
        var result = DouyinProbeRequestFactory.TryCreate(
            Config(),
            name => name switch
            {
                "AUTOLIVE_DOUYIN_ROOT" => "C:\\Douyin_Spider",
                "AUTOLIVE_DOUYIN_PROBE" => "C:\\probe.py",
                "CONDA_EXE" => "C:\\conda.exe",
                "AUTOLIVE_DOUYIN_TIMEOUT_SEC" => "30.5",
                _ => null
            },
            "C:\\Temp",
            out _,
            out var error,
            out _);

        Assert.IsFalse(result);
        StringAssert.Contains(error!, "30～900");
    }

    private static DouyinLiveConfig Config() => new()
    {
        Enabled = true,
        RoomId = "12345",
        Replies = ["收到"],
        QueueCapacity = DouyinLiveRules.DefaultQueueCapacity
    };
}
