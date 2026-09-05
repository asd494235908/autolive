using System.Text.Json;
using GpAutoLive.Contracts;
using GpAutoLive.Media;

namespace GpAutoLive.Media.Tests;

[TestClass]
public sealed class SyncFixtureContractTests
{
    [TestMethod]
    public void Shared_fixtures_match_csharp_contract_boundaries()
    {
        var media = Read("media-effects.json");
        CollectionAssert.AreEqual(
            new[] { "gpu83", "cpu4", "original" },
            media.GetProperty("expected").GetProperty("fallback_order")
                .EnumerateArray()
                .Select(static item => item.GetString())
                .ToArray());
        Assert.IsTrue(MpvVideoEffectSnapshot.TryCreate(
            MpvVideoProcessingMode.Gpu83,
            media.GetProperty("input").GetProperty("brightness_percent").GetDouble(),
            media.GetProperty("input").GetProperty("contrast_percent").GetDouble(),
            media.GetProperty("input").GetProperty("saturation_percent").GetDouble(),
            media.GetProperty("input").GetProperty("hue_degrees").GetDouble(),
            MpvShaderOptionsSnapshot.Empty,
            out var effect,
            out var effectError), effectError?.Message);
        Assert.AreEqual(MpvVideoProcessingMode.Gpu83, effect?.Mode);

        var playback = Read("playback-state.json");
        Assert.AreEqual(1, playback.GetProperty("expected").GetProperty("next_active_index").GetInt32());
        Assert.IsTrue(playback.GetProperty("expected").GetProperty("window_reused").GetBoolean());
        Assert.IsFalse(playback.GetProperty("expected").GetProperty("generated_version_file").GetBoolean());

        var audio = Read("audio-candidate.json");
        CollectionAssert.AreEquivalent(
            new[] { "portaudio", "rtmp" },
            audio.GetProperty("input").GetProperty("final_pcm_consumers")
                .EnumerateArray()
                .Select(static item => item.GetString())
                .ToArray());
        Assert.IsFalse(audio.GetProperty("input").GetProperty("rtmp_consumer_owns_bus").GetBoolean());
        Assert.IsFalse(audio.GetProperty("expected").GetProperty("duplicate_source_decode").GetBoolean());

        var rtmp = Read("rtmp-output.json");
        var targetScheme = rtmp.GetProperty("input").GetProperty("target_scheme").GetString();
        var rtmpConfig = new RtmpOutputConfig
        {
            TargetUrl = $"{targetScheme}://127.0.0.1/live/fixture",
            VideoEnabled = true,
            AudioEnabled = true,
        };
        Assert.IsTrue(RtmpOutputRules.TryValidate(rtmpConfig, out var rtmpError), rtmpError?.Message);
        Assert.IsFalse(rtmp.GetProperty("expected").GetProperty("video_capture").GetBoolean());

        var virtualCamera = Read("virtual-camera.json");
        Assert.AreEqual("GpAutoLive Camera", virtualCamera.GetProperty("input").GetProperty("device_name").GetString());
        Assert.AreEqual("YUY2", virtualCamera.GetProperty("input").GetProperty("pixel_format").GetString());
        Assert.AreEqual(1280, virtualCamera.GetProperty("input").GetProperty("width").GetInt32());
        Assert.AreEqual(720, virtualCamera.GetProperty("input").GetProperty("height").GetInt32());
        Assert.AreEqual(30, virtualCamera.GetProperty("input").GetProperty("fps").GetInt32());
        Assert.IsFalse(virtualCamera.GetProperty("input").GetProperty("zero_copy").GetBoolean());

        var douyin = Read("douyin-m1.json");
        Assert.AreEqual("WebcastChatMessage", douyin.GetProperty("input").GetProperty("event_type").GetString());
        Assert.AreEqual(0, douyin.GetProperty("expected").GetProperty("model_calls").GetInt32());
        Assert.AreEqual(0, douyin.GetProperty("expected").GetProperty("go_calls").GetInt32());
        Assert.IsFalse(douyin.GetProperty("expected").GetProperty("credential_persisted").GetBoolean());
    }

    private static JsonElement Read(string fileName)
    {
        var path = Path.Combine(AppContext.BaseDirectory, "SyncFixtures", fileName);
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        return document.RootElement.Clone();
    }
}
