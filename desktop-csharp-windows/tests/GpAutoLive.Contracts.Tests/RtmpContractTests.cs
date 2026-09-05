using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Contracts.Tests;

[TestClass]
public sealed class RtmpContractTests
{
    [TestMethod]
    public void Validates_rtmp_and_rtmps_without_network_access()
    {
        var config = RtmpOutputConfig.Default with
        {
            TargetUrl = "rtmp://127.0.0.1:1935/live/stream?token=opaque"
        };

        Assert.IsTrue(RtmpOutputRules.TryValidate(config, out var error), error?.Message);
        Assert.IsNull(error);
        Assert.AreEqual(
            "rtmp://127.0.0.1:1935/<redacted>",
            RtmpOutputRules.RedactTargetUrl(config.TargetUrl));

        Assert.AreEqual(
            "rtmps://[::1]:443/<redacted>",
            RtmpOutputRules.RedactTargetUrl("rtmps://[::1]:443/live/stream"));
    }

    [TestMethod]
    public void Rejects_credentials_invalid_tracks_and_odd_video_size()
    {
        var config = RtmpOutputConfig.Default with
        {
            TargetUrl = "rtmp://user:password@127.0.0.1/live/stream"
        };
        Assert.IsFalse(RtmpOutputRules.TryValidate(config, out var error));
        Assert.AreEqual(RtmpConfigFailureCode.TargetUrlInvalid, error?.Code);

        config = config with
        {
            TargetUrl = "rtmp://127.0.0.1/live/stream",
            VideoEnabled = false,
            AudioEnabled = false
        };
        Assert.IsFalse(RtmpOutputRules.TryValidate(config, out error));
        Assert.AreEqual(RtmpConfigFailureCode.TrackRequired, error?.Code);

        config = config with
        {
            AudioEnabled = true,
            VideoEnabled = true,
            Width = 1_281
        };
        Assert.IsFalse(RtmpOutputRules.TryValidate(config, out error));
        Assert.AreEqual(RtmpConfigFailureCode.VideoSizeOutOfRange, error?.Code);
    }

    [TestMethod]
    public void Serializes_with_snake_case_and_redacts_only_status_value()
    {
        var config = RtmpOutputConfig.Default with
        {
            TargetUrl = "rtmps://media.example.com/live/stream?token=secret"
        };
        var json = JsonSerializer.Serialize(config, ContractJson.CreateOptions());

        StringAssert.Contains(json, "\"target_url\"");
        StringAssert.Contains(json, "token=secret");
        Assert.IsFalse(json.Contains("TargetUrl", StringComparison.Ordinal));

        var snapshot = new RtmpOutputSnapshot(
            RtmpOutputState.Starting,
            1,
            RtmpOutputRules.RedactTargetUrl(config.TargetUrl),
            true,
            true,
            1280,
            720,
            30,
            2500,
            128,
            "h264_amf",
            null,
            0,
            null,
            null);
        var snapshotJson = JsonSerializer.Serialize(snapshot, ContractJson.CreateOptions());
        Assert.IsFalse(snapshotJson.Contains("token=secret", StringComparison.Ordinal));
        var roundTripped = JsonSerializer.Deserialize<RtmpOutputSnapshot>(
            snapshotJson,
            ContractJson.CreateOptions());
        Assert.AreEqual("rtmps://media.example.com/<redacted>", roundTripped?.TargetUrl);
    }

    [TestMethod]
    public void Encoder_order_only_moves_forward()
    {
        CollectionAssert.AreEqual(
            new[] { "h264_amf", "h264_qsv", "h264_mf", "libopenh264" },
            RtmpOutputRules.EncoderAttemptOrder("h264_amf").ToArray());
        CollectionAssert.AreEqual(
            RtmpOutputRules.H264EncoderOrder.ToArray(),
            RtmpOutputRules.EncoderAttemptOrder("unknown").ToArray());
    }
}
