using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Contracts.Tests;

[TestClass]
public sealed class ContractJsonTests
{
    [TestMethod]
    public void Source_media_uses_wire_field_names()
    {
        var source = new SourceMediaDto(
            SourcePath: "C:\\media\\sample.mp4",
            PlaybackReference: "C:\\media\\sample.mp4",
            MediaKind: MediaKind.Video,
            CompatibilityMode: MediaCompatibilityMode.Direct,
            FileName: "sample.mp4",
            FileSizeBytes: 42,
            DurationMs: 1_000,
            AudioStartMs: null,
            AudioEndMs: null,
            Width: 1280,
            Height: 720,
            FrameRateFps: 30,
            AudioSampleRateHz: 48_000,
            AudioChannelCount: 2,
            VideoCodecName: "h264",
            AudioCodecName: "aac",
            Mp4Sha256: null,
            Mp4HashStatus: "disabled");

        var json = JsonSerializer.Serialize(source, ContractJson.CreateOptions());

        StringAssert.Contains(json, "\"source_path\"");
        StringAssert.Contains(json, "\"media_kind\":\"video\"");
        Assert.IsFalse(json.Contains("SourcePath", StringComparison.Ordinal));
    }

    [TestMethod]
    public void Unknown_json_fields_are_rejected()
    {
        var json = "{\"path\":\"C:\\\\sample.mp4\",\"unexpected\":true}";

        Assert.ThrowsExactly<JsonException>(() =>
            JsonSerializer.Deserialize<MediaProbeRequestDto>(json, ContractJson.CreateOptions()));
    }

    [TestMethod]
    public void Auth_dtos_use_snake_case_and_keep_error_shape_stable()
    {
        var request = new ActivateDeviceRequestDto(
            new DeviceRegistrationDto("autolive", "device01", "Windows desktop", "windows", "1.0.0"));
        var error = new ControlPlaneErrorDto("DEVICE_DISABLED", "设备已被禁用", 403, "req-1", 10);

        var json = JsonSerializer.Serialize(new { request, error }, ContractJson.CreateOptions());

        StringAssert.Contains(json, "\"device_id\"");
        StringAssert.Contains(json, "\"request_id\"");
        StringAssert.Contains(json, "\"retry_after_seconds\"");
        Assert.IsFalse(json.Contains("DeviceId", StringComparison.Ordinal));
    }

    [TestMethod]
    public void Auth_input_validation_rejects_short_device_and_wrong_audience()
    {
        Assert.IsFalse(AuthContractValidation.TryValidateDeviceId("short", out var deviceError));
        Assert.AreEqual(AuthErrorCodes.InvalidRequest, deviceError!.Code);

        var tokens = new SessionTokensDto("access", "refresh", "2099-01-01T00:00:00Z", "admin");
        Assert.IsFalse(AuthContractValidation.TryValidateSessionTokens(tokens, DateTimeOffset.UtcNow, out var tokenError));
        Assert.AreEqual(AuthErrorCodes.ResponseInvalid, tokenError!.Code);

        var oversizedUtf8Password = new DesktopLoginRequestDto("alice", "密".PadRight(86, '密'));
        Assert.IsFalse(AuthContractValidation.TryValidateLogin(oversizedUtf8Password, out var passwordError));
        Assert.AreEqual(AuthErrorCodes.InvalidRequest, passwordError!.Code);
    }

    [TestMethod]
    public void Heartbeat_input_validation_bounds_negative_metrics_and_nul()
    {
        var request = new HeartbeatRequestDto(
            "autolive",
            "device01",
            DateTimeOffset.UtcNow,
            new HeartbeatStatusDto(-1, CurrentMediaName: "bad\0name"));

        Assert.IsFalse(AuthContractValidation.TryValidateHeartbeat(request, out var error));
        Assert.AreEqual(AuthErrorCodes.InvalidRequest, error!.Code);
    }

    [TestMethod]
    public void Heartbeat_playback_state_only_accepts_server_contract_values()
    {
        foreach (var playbackState in new string?[] { null, string.Empty, "idle", "playing", "paused", "error" })
        {
            var request = new HeartbeatRequestDto(
                ControlPlaneContractValues.Product,
                "device01",
                DateTimeOffset.UtcNow,
                new HeartbeatStatusDto(0, PlaybackState: playbackState));

            Assert.IsTrue(
                AuthContractValidation.TryValidateHeartbeat(request, out var error),
                $"服务端允许的心跳播放状态被拒绝：{playbackState ?? "<null>"}，{error?.Message}");
        }

        foreach (var playbackState in new[] { "ready", "stopped", "buffering" })
        {
            var request = new HeartbeatRequestDto(
                ControlPlaneContractValues.Product,
                "device01",
                DateTimeOffset.UtcNow,
                new HeartbeatStatusDto(0, PlaybackState: playbackState));

            Assert.IsFalse(
                AuthContractValidation.TryValidateHeartbeat(request, out var error),
                $"非服务端契约的心跳播放状态未被拒绝：{playbackState}");
            Assert.AreEqual(AuthErrorCodes.InvalidRequest, error!.Code);
        }
    }
}
