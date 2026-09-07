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
        AssertFixtureMetadata(media, "media-effects");
        CollectionAssert.AreEqual(
            new[] { "gpu83", "cpu4", "original" },
            media.GetProperty("expected").GetProperty("fallback_order")
                .EnumerateArray()
                .Select(static item => item.GetString())
                .ToArray());
        Assert.IsTrue(media.GetProperty("expected").GetProperty("accepted").GetBoolean());
        Assert.IsNull(media.GetProperty("expected").GetProperty("error_code").GetString());
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
        AssertFixtureMetadata(playback, "playback-state");
        CollectionAssert.AreEqual(
            new[] { "media-a", "media-b" },
            playback.GetProperty("input").GetProperty("pool")
                .EnumerateArray()
                .Select(static item => item.GetString())
                .ToArray());
        Assert.AreEqual("eof", playback.GetProperty("input").GetProperty("event").GetString());
        Assert.AreEqual(1, playback.GetProperty("expected").GetProperty("next_active_index").GetInt32());
        Assert.IsTrue(playback.GetProperty("expected").GetProperty("window_reused").GetBoolean());
        Assert.IsFalse(playback.GetProperty("expected").GetProperty("pool_order_changed").GetBoolean());
        Assert.IsFalse(playback.GetProperty("expected").GetProperty("generated_version_file").GetBoolean());
        Assert.IsNull(playback.GetProperty("expected").GetProperty("error_code").GetString());

        var audio = Read("audio-candidate.json");
        AssertFixtureMetadata(audio, "audio-candidate");
        Assert.AreEqual("N+1", audio.GetProperty("expected").GetProperty("selected_candidate").GetString());
        CollectionAssert.AreEquivalent(
            new[] { "portaudio", "rtmp" },
            audio.GetProperty("input").GetProperty("final_pcm_consumers")
                .EnumerateArray()
                .Select(static item => item.GetString())
                .ToArray());
        Assert.IsFalse(audio.GetProperty("input").GetProperty("rtmp_consumer_owns_bus").GetBoolean());
        Assert.IsTrue(audio.GetProperty("expected").GetProperty("portaudio_consumer_preserved").GetBoolean());
        Assert.IsTrue(audio.GetProperty("expected").GetProperty("rtmp_consumer_preserved").GetBoolean());
        Assert.IsFalse(audio.GetProperty("expected").GetProperty("duplicate_source_decode").GetBoolean());
        Assert.IsNull(audio.GetProperty("expected").GetProperty("error_code").GetString());

        var rtmp = Read("rtmp-output.json");
        AssertFixtureMetadata(rtmp, "rtmp-output");
        var targetScheme = rtmp.GetProperty("input").GetProperty("target_scheme").GetString();
        CollectionAssert.AreEquivalent(
            new[] { "video", "audio" },
            rtmp.GetProperty("input").GetProperty("tracks")
                .EnumerateArray()
                .Select(static item => item.GetString())
                .ToArray());
        CollectionAssert.AreEqual(
            new long[] { 1, 2, 4, 8, 15 },
            rtmp.GetProperty("expected").GetProperty("retry_seconds")
                .EnumerateArray()
                .Select(static item => item.GetInt64())
                .ToArray());
        var rtmpConfig = new RtmpOutputConfig
        {
            TargetUrl = $"{targetScheme}://127.0.0.1/live/fixture",
            VideoEnabled = true,
            AudioEnabled = true,
        };
        Assert.IsTrue(RtmpOutputRules.TryValidate(rtmpConfig, out var rtmpError), rtmpError?.Message);
        Assert.IsTrue(rtmp.GetProperty("input").GetProperty("source_is_direct_media").GetBoolean());
        Assert.AreEqual("final_pcm_bus", rtmp.GetProperty("input").GetProperty("audio_source").GetString());
        Assert.IsFalse(rtmp.GetProperty("expected").GetProperty("video_capture").GetBoolean());
        Assert.IsFalse(rtmp.GetProperty("expected").GetProperty("audio_decode_duplicate").GetBoolean());
        Assert.IsNull(rtmp.GetProperty("expected").GetProperty("error_code").GetString());

        var virtualCamera = Read("virtual-camera.json");
        AssertFixtureMetadata(virtualCamera, "virtual-camera");
        Assert.AreEqual("GpAutoLive Camera", virtualCamera.GetProperty("input").GetProperty("device_name").GetString());
        Assert.AreEqual("YUY2", virtualCamera.GetProperty("input").GetProperty("pixel_format").GetString());
        Assert.AreEqual(1280, virtualCamera.GetProperty("input").GetProperty("width").GetInt32());
        Assert.AreEqual(720, virtualCamera.GetProperty("input").GetProperty("height").GetInt32());
        Assert.AreEqual(30, virtualCamera.GetProperty("input").GetProperty("fps").GetInt32());
        Assert.IsFalse(virtualCamera.GetProperty("input").GetProperty("zero_copy").GetBoolean());
        Assert.AreEqual("final_effect_hwnd", virtualCamera.GetProperty("input").GetProperty("capture_scope").GetString());
        Assert.IsFalse(virtualCamera.GetProperty("expected").GetProperty("cpu_full_frame_conversion").GetBoolean());
        Assert.IsFalse(virtualCamera.GetProperty("expected").GetProperty("warp_adapter_allowed").GetBoolean());
        Assert.IsTrue(virtualCamera.GetProperty("expected").GetProperty("sidecar_required").GetBoolean());
        Assert.IsNull(virtualCamera.GetProperty("expected").GetProperty("error_code").GetString());

        var douyin = Read("douyin-m1.json");
        AssertFixtureMetadata(douyin, "douyin-m1");
        Assert.AreEqual("WebcastChatMessage", douyin.GetProperty("input").GetProperty("event_type").GetString());
        Assert.IsTrue(douyin.GetProperty("expected").GetProperty("accepted").GetBoolean());
        Assert.AreEqual("local_pool", douyin.GetProperty("expected").GetProperty("reply_source").GetString());
        Assert.AreEqual("serial_bounded_queue", douyin.GetProperty("expected").GetProperty("send_mode").GetString());
        Assert.IsFalse(douyin.GetProperty("expected").GetProperty("expired").GetBoolean());
        Assert.AreEqual(0, douyin.GetProperty("expected").GetProperty("model_calls").GetInt32());
        Assert.AreEqual(0, douyin.GetProperty("expected").GetProperty("go_calls").GetInt32());
        Assert.IsFalse(douyin.GetProperty("expected").GetProperty("credential_persisted").GetBoolean());
        Assert.IsNull(douyin.GetProperty("expected").GetProperty("error_code").GetString());

        var errors = Read("error-codes.json");
        AssertFixtureMetadata(errors, "cross-client-error-categories");
        var expectedErrors = new Dictionary<string, (bool Retryable, string UiState)>(StringComparer.Ordinal)
        {
            ["invalid_input"] = (false, "error"),
            ["not_ready"] = (false, "pending"),
            ["resource_unavailable"] = (true, "degraded"),
            ["external_process_failed"] = (true, "error"),
            ["external_service_rejected"] = (false, "error"),
            ["cancelled"] = (false, "cancelled"),
            ["stale_generation"] = (false, "ignored"),
            ["ownership_conflict"] = (false, "blocked"),
        };
        var actualErrors = errors.GetProperty("codes")
            .EnumerateArray()
            .ToDictionary(
                item => item.GetProperty("code").GetString()!,
                item => (
                    item.GetProperty("retryable").GetBoolean(),
                    item.GetProperty("ui_state").GetString()!),
                StringComparer.Ordinal);
        CollectionAssert.AreEquivalent(expectedErrors.Keys.ToArray(), actualErrors.Keys.ToArray());
        foreach (var (code, expected) in expectedErrors)
        {
            Assert.AreEqual(expected.Retryable, actualErrors[code].Item1, code);
            Assert.AreEqual(expected.UiState, actualErrors[code].Item2, code);
        }
    }

    private static void AssertFixtureMetadata(JsonElement document, string expectedContract)
    {
        Assert.AreEqual(1, document.GetProperty("schema_version").GetInt32());
        Assert.AreEqual(expectedContract, document.GetProperty("contract").GetString());
    }

    private static JsonElement Read(string fileName)
    {
        var path = Path.Combine(AppContext.BaseDirectory, "SyncFixtures", fileName);
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        return document.RootElement.Clone();
    }
}
