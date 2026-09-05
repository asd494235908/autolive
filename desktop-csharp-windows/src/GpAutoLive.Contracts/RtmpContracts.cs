using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>RTMP/RTMPS 输出状态；外部进程和 UI 只消费该有限状态集。</summary>
public enum RtmpOutputState
{
    /// <summary>未启动。</summary>
    Idle,
    /// <summary>正在启动受管 FFmpeg。</summary>
    Starting,
    /// <summary>已发布。</summary>
    Publishing,
    /// <summary>连接断开后的有限重连阶段。</summary>
    Reconnecting,
    /// <summary>正在停止。</summary>
    Stopping,
    /// <summary>不可恢复或重试耗尽。</summary>
    Failed,
}

/// <summary>RTMP 配置校验失败分类。</summary>
public enum RtmpConfigFailureCode
{
    /// <summary>地址为空。</summary>
    TargetUrlRequired,
    /// <summary>地址超过长度上限。</summary>
    TargetUrlTooLong,
    /// <summary>地址包含首尾空格。</summary>
    TargetUrlWhitespace,
    /// <summary>地址格式无效。</summary>
    TargetUrlInvalid,
    /// <summary>协议不是 rtmp/rtmps。</summary>
    UnsupportedScheme,
    /// <summary>缺少发布路径。</summary>
    TargetUrlPathRequired,
    /// <summary>主机无效。</summary>
    TargetUrlHostInvalid,
    /// <summary>端口无效。</summary>
    TargetUrlPortInvalid,
    /// <summary>未选择任何轨道。</summary>
    TrackRequired,
    /// <summary>视频尺寸无效。</summary>
    VideoSizeOutOfRange,
    /// <summary>帧率不在白名单。</summary>
    FpsOutOfRange,
    /// <summary>视频码率无效。</summary>
    VideoBitrateOutOfRange,
    /// <summary>音频码率无效。</summary>
    AudioBitrateOutOfRange,
}

/// <summary>可安全投影到界面的 RTMP 配置错误。</summary>
public sealed record RtmpConfigError(
    RtmpConfigFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>
/// RTMP/RTMPS 发布配置。TargetUrl 仅在内存中使用，任何状态快照都必须调用脱敏方法。
/// </summary>
public sealed record RtmpOutputConfig
{
    /// <summary>用户提供的 RTMP/RTMPS 地址；仅允许在内存中使用。</summary>
    [property: JsonPropertyName("target_url")]
    public string TargetUrl { get; init; } = string.Empty;

    /// <summary>是否发布画面。</summary>
    [property: JsonPropertyName("video_enabled")]
    public bool VideoEnabled { get; init; } = true;

    /// <summary>是否发布声音。</summary>
    [property: JsonPropertyName("audio_enabled")]
    public bool AudioEnabled { get; init; } = true;

    /// <summary>输出宽度。</summary>
    [property: JsonPropertyName("width")]
    public uint Width { get; init; } = 1_280;

    /// <summary>输出高度。</summary>
    [property: JsonPropertyName("height")]
    public uint Height { get; init; } = 720;

    /// <summary>输出帧率。</summary>
    [property: JsonPropertyName("fps")]
    public uint Fps { get; init; } = 30;

    /// <summary>视频码率，单位为 kbps。</summary>
    [property: JsonPropertyName("video_bitrate_kbps")]
    public uint VideoBitrateKbps { get; init; } = 2_500;

    /// <summary>音频码率，单位为 kbps。</summary>
    [property: JsonPropertyName("audio_bitrate_kbps")]
    public uint AudioBitrateKbps { get; init; } = 128;

    /// <summary>与现有桌面端面板一致的默认值；目标地址仍要求用户显式填写。</summary>
    public static RtmpOutputConfig Default { get; } = new();
}

/// <summary>RTMP 发布时绑定的播放池身份；不包含路径或媒体正文。</summary>
public sealed record RtmpSourceIdentity(
    [property: JsonPropertyName("playback_generation")] ulong PlaybackGeneration,
    [property: JsonPropertyName("source_revision")] ulong SourceRevision,
    [property: JsonPropertyName("source_media_index")] int SourceMediaIndex,
    [property: JsonPropertyName("loop_index")] ulong LoopIndex,
    [property: JsonPropertyName("source_position_ms")] ulong SourcePositionMs,
    [property: JsonPropertyName("source_duration_ms")] ulong? SourceDurationMs);

/// <summary>RTMP 对外状态；TargetUrl 始终为脱敏地址。</summary>
public sealed record RtmpOutputSnapshot(
    [property: JsonPropertyName("state")] RtmpOutputState State,
    [property: JsonPropertyName("session_generation")] ulong SessionGeneration,
    [property: JsonPropertyName("target_url")] string? TargetUrl,
    [property: JsonPropertyName("video_enabled")] bool VideoEnabled,
    [property: JsonPropertyName("audio_enabled")] bool AudioEnabled,
    [property: JsonPropertyName("width")] uint? Width,
    [property: JsonPropertyName("height")] uint? Height,
    [property: JsonPropertyName("fps")] uint? Fps,
    [property: JsonPropertyName("video_bitrate_kbps")] uint? VideoBitrateKbps,
    [property: JsonPropertyName("audio_bitrate_kbps")] uint? AudioBitrateKbps,
    [property: JsonPropertyName("encoder")] string? Encoder,
    [property: JsonPropertyName("source_identity")] RtmpSourceIdentity? SourceIdentity,
    [property: JsonPropertyName("retry_count")] int RetryCount,
    [property: JsonPropertyName("error_code")] string? ErrorCode,
    [property: JsonPropertyName("error")] string? Error);

/// <summary>RTMP 配置验证、地址脱敏和编码器候选顺序。</summary>
public static class RtmpOutputRules
{
    /// <summary>目标地址最大字符数。</summary>
    public const int MaxTargetUrlLength = 2_048;
    /// <summary>FFmpeg 参数最大数量。</summary>
    public const int MaxArgumentCount = 64;
    /// <summary>单个 FFmpeg 参数最大字符数。</summary>
    public const int MaxArgumentCharacters = 32_000;
    /// <summary>最终 PCM 输入采样率。</summary>
    public const uint AudioSampleRateHz = 48_000;
    /// <summary>允许的固定视频帧率。</summary>
    public static readonly IReadOnlyList<uint> AllowedFps = [25, 30, 50, 60];
    /// <summary>Windows H.264 编码器从高到低的候选顺序。</summary>
    public static readonly IReadOnlyList<string> H264EncoderOrder =
        ["h264_nvenc", "h264_amf", "h264_qsv", "h264_mf", "libopenh264"];

    /// <summary>校验用户可编辑的 RTMP 配置；不访问网络。</summary>
    public static bool TryValidate(
        RtmpOutputConfig? config,
        out RtmpConfigError? error)
    {
        error = null;
        if (config is null)
        {
            error = Invalid(RtmpConfigFailureCode.TargetUrlRequired, "RTMP 地址不能为空");
            return false;
        }

        var targetUrl = config.TargetUrl;
        if (string.IsNullOrWhiteSpace(targetUrl))
        {
            error = Invalid(RtmpConfigFailureCode.TargetUrlRequired, "RTMP 地址不能为空");
            return false;
        }

        if (targetUrl.Length > MaxTargetUrlLength)
        {
            error = Invalid(RtmpConfigFailureCode.TargetUrlTooLong, "RTMP 地址过长");
            return false;
        }

        if (!string.Equals(targetUrl, targetUrl.Trim(), StringComparison.Ordinal))
        {
            error = Invalid(RtmpConfigFailureCode.TargetUrlWhitespace, "RTMP 地址首尾不能包含空格");
            return false;
        }

        if (targetUrl.Any(static character =>
                char.IsControl(character) || char.IsWhiteSpace(character) || character == '\\'))
        {
            error = Invalid(RtmpConfigFailureCode.TargetUrlInvalid, "RTMP 地址格式无效");
            return false;
        }

        if (!Uri.TryCreate(targetUrl, UriKind.Absolute, out var uri)
            || uri is null)
        {
            error = Invalid(RtmpConfigFailureCode.TargetUrlInvalid, "RTMP 地址格式无效");
            return false;
        }

        if (!string.Equals(uri.Scheme, "rtmp", StringComparison.OrdinalIgnoreCase)
            && !string.Equals(uri.Scheme, "rtmps", StringComparison.OrdinalIgnoreCase))
        {
            error = Invalid(RtmpConfigFailureCode.UnsupportedScheme, "仅支持 rtmp:// 或 rtmps://");
            return false;
        }

        if (!string.IsNullOrEmpty(uri.UserInfo)
            || !string.IsNullOrEmpty(uri.Fragment))
        {
            error = Invalid(RtmpConfigFailureCode.TargetUrlInvalid, "RTMP 地址格式无效");
            return false;
        }

        if (string.IsNullOrEmpty(uri.Host))
        {
            error = Invalid(RtmpConfigFailureCode.TargetUrlHostInvalid, "RTMP 地址主机无效");
            return false;
        }

        if (uri.Port is 0 or < -1 or > 65_535)
        {
            error = Invalid(RtmpConfigFailureCode.TargetUrlPortInvalid, "RTMP 地址端口无效");
            return false;
        }

        var path = uri.AbsolutePath.Trim('/');
        if (path.Length == 0)
        {
            error = Invalid(RtmpConfigFailureCode.TargetUrlPathRequired, "RTMP 地址必须包含发布路径");
            return false;
        }

        if (!config.VideoEnabled && !config.AudioEnabled)
        {
            error = Invalid(RtmpConfigFailureCode.TrackRequired, "至少选择画面或声音一项");
            return false;
        }

        if (config.VideoEnabled
            && (config.Width is < 16 or > 7_680
                || config.Height is < 16 or > 4_320
                || !config.Width.IsEven()
                || !config.Height.IsEven()))
        {
            error = Invalid(RtmpConfigFailureCode.VideoSizeOutOfRange, "视频尺寸超出允许范围");
            return false;
        }

        if (config.VideoEnabled && !AllowedFps.Contains(config.Fps))
        {
            error = Invalid(RtmpConfigFailureCode.FpsOutOfRange, "帧率仅支持 25、30、50 或 60 FPS");
            return false;
        }

        if (config.VideoEnabled && config.VideoBitrateKbps is < 64 or > 100_000)
        {
            error = Invalid(RtmpConfigFailureCode.VideoBitrateOutOfRange, "视频码率超出允许范围");
            return false;
        }

        if (config.AudioEnabled && config.AudioBitrateKbps is < 16 or > 512)
        {
            error = Invalid(RtmpConfigFailureCode.AudioBitrateOutOfRange, "声音码率超出允许范围");
            return false;
        }

        return true;
    }

    /// <summary>只保留 scheme、authority 和固定占位符，不泄露 stream key/query。</summary>
    public static string RedactTargetUrl(string? targetUrl)
    {
        if (string.IsNullOrWhiteSpace(targetUrl)
            || !Uri.TryCreate(targetUrl, UriKind.Absolute, out var uri)
            || uri is null
            || string.IsNullOrEmpty(uri.Host))
        {
            return "<redacted>";
        }

        var host = uri.DnsSafeHost.Trim('[', ']');
        var authority = uri.HostNameType == UriHostNameType.IPv6
            ? $"[{host}]"
            : host;
        if (!uri.IsDefaultPort && uri.Port > 0)
        {
            authority = $"{authority}:{uri.Port}";
        }

        return $"{uri.Scheme}://{authority}/<redacted>";
    }

    /// <summary>从首选编码器开始单向向低能力候选降级，不允许回跳。</summary>
    public static IReadOnlyList<string> EncoderAttemptOrder(string? preferred)
    {
        var start = string.IsNullOrWhiteSpace(preferred)
            ? 0
            : Array.IndexOf(H264EncoderOrder.ToArray(), preferred);
        if (start < 0)
        {
            start = 0;
        }

        return H264EncoderOrder.Skip(start).ToArray();
    }

    private static RtmpConfigError Invalid(RtmpConfigFailureCode code, string message) =>
        new(code, message);

    private static bool IsEven(this uint value) => value % 2 == 0;
}
