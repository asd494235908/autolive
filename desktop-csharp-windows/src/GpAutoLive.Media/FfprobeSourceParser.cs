using System.Globalization;
using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Media;

/// <summary>
/// 将受管 FFprobe 的最小 JSON 结果转换为统一媒体 DTO。
/// 解析只消费内存字符串，不保存原始 JSON 或外部错误输出。
/// </summary>
public static class FfprobeSourceParser
{
    private const int MaxCodecNameLength = 64;

    public static bool TryParse(
        ValidatedMediaPath mediaPath,
        ulong fileSizeBytes,
        string? probeJson,
        out SourceMediaDto? source,
        out MediaProbeError? error)
    {
        source = null;
        error = null;

        if (fileSizeBytes == 0 || string.IsNullOrWhiteSpace(probeJson))
        {
            error = InvalidMetadata();
            return false;
        }

        var pathError = MediaPathPolicy.Validate(
            mediaPath.CanonicalPath,
            requireExistingFile: false,
            out var validatedPath);
        if (pathError is not null || validatedPath.Kind != mediaPath.Kind)
        {
            error = pathError ?? UnsupportedStream();
            return false;
        }

        mediaPath = validatedPath;

        try
        {
            using var document = JsonDocument.Parse(probeJson);
            if (document.RootElement.ValueKind != JsonValueKind.Object
                || !document.RootElement.TryGetProperty("format", out var format)
                || format.ValueKind != JsonValueKind.Object
                || !TryParseDurationMs(format, out var durationMs))
            {
                error = InvalidMetadata();
                return false;
            }

            if (!document.RootElement.TryGetProperty("streams", out var streams)
                || streams.ValueKind != JsonValueKind.Array)
            {
                error = UnsupportedStream();
                return false;
            }

            JsonElement? video = null;
            JsonElement? audio = null;
            foreach (var stream in streams.EnumerateArray())
            {
                if (stream.ValueKind != JsonValueKind.Object
                    || !TryGetString(stream, "codec_type", out var codecType))
                {
                    continue;
                }

                if (codecType.Equals("video", StringComparison.OrdinalIgnoreCase)
                    && !IsAttachedPicture(stream)
                    && video is null)
                {
                    video = stream;
                }
                else if (codecType.Equals("audio", StringComparison.OrdinalIgnoreCase) && audio is null)
                {
                    audio = stream;
                }
            }

            if (mediaPath.Kind is MediaKind.Video)
            {
                if (video is null || !TryParseVideo(video.Value, out var videoData, out error))
                {
                    error ??= UnsupportedStream();
                    return false;
                }

                source = new SourceMediaDto(
                    mediaPath.CanonicalPath,
                    mediaPath.CanonicalPath,
                    MediaKind.Video,
                    MediaCompatibilityMode.Direct,
                    Path.GetFileName(mediaPath.CanonicalPath),
                    fileSizeBytes,
                    durationMs,
                    null,
                    null,
                    videoData.Width,
                    videoData.Height,
                    videoData.FrameRate,
                    audio is null ? null : ParseSampleRate(audio.Value),
                    audio is null ? null : ParseChannelCount(audio.Value),
                    videoData.CodecName,
                    audio is null ? null : ParseCodecName(audio.Value),
                    null,
                    "disabled");
                return true;
            }

            if (audio is null || video is not null || !TryParseAudio(audio.Value, out var audioData, out error))
            {
                error ??= UnsupportedStream();
                return false;
            }

            source = new SourceMediaDto(
                mediaPath.CanonicalPath,
                mediaPath.CanonicalPath,
                MediaKind.Audio,
                MediaCompatibilityMode.Direct,
                Path.GetFileName(mediaPath.CanonicalPath),
                fileSizeBytes,
                durationMs,
                null,
                null,
                null,
                null,
                null,
                audioData.SampleRate,
                audioData.ChannelCount,
                null,
                audioData.CodecName,
                null,
                "disabled");
            return true;
        }
        catch (JsonException)
        {
            error = new MediaProbeError(
                MediaProbeFailureCode.MalformedProbeOutput,
                "媒体探测返回的数据格式无效。",
                Retryable: false);
            return false;
        }
    }

    private static bool TryParseVideo(
        JsonElement stream,
        out (uint Width, uint Height, double FrameRate, string CodecName) value,
        out MediaProbeError? error)
    {
        value = default;
        error = null;
        if (!TryGetPositiveUInt(stream, "width", out var width)
            || !TryGetPositiveUInt(stream, "height", out var height)
            || !TryGetFrameRate(stream, out var frameRate)
            || !TryGetCodecName(stream, out var codecName))
        {
            error = InvalidMetadata();
            return false;
        }

        value = (width, height, frameRate, codecName);
        return true;
    }

    private static bool TryParseAudio(
        JsonElement stream,
        out (uint SampleRate, ushort ChannelCount, string CodecName) value,
        out MediaProbeError? error)
    {
        value = default;
        error = null;
        if (!TryGetPositiveUInt(stream, "sample_rate", out var sampleRate)
            || sampleRate > uint.MaxValue
            || !TryGetPositiveUShort(stream, "channels", out var channels)
            || !TryGetCodecName(stream, out var codecName))
        {
            error = InvalidMetadata();
            return false;
        }

        value = (sampleRate, channels, codecName);
        return true;
    }

    private static bool TryParseDurationMs(JsonElement format, out ulong durationMs)
    {
        durationMs = 0;
        if (!TryGetDouble(format, "duration", out var seconds)
            || !double.IsFinite(seconds)
            || seconds <= 0)
        {
            return false;
        }

        var milliseconds = seconds * 1000d;
        if (!double.IsFinite(milliseconds)
            || milliseconds < 1
            || milliseconds >= ulong.MaxValue)
        {
            return false;
        }

        durationMs = (ulong)Math.Round(milliseconds, MidpointRounding.AwayFromZero);
        return durationMs > 0;
    }

    private static bool TryGetFrameRate(JsonElement stream, out double frameRate)
    {
        if (TryParseFrameRate(stream, "avg_frame_rate", out frameRate)
            || TryParseFrameRate(stream, "r_frame_rate", out frameRate))
        {
            return true;
        }

        frameRate = 0;
        return false;
    }

    private static bool TryParseFrameRate(JsonElement stream, string propertyName, out double frameRate)
    {
        frameRate = 0;
        if (!TryGetString(stream, propertyName, out var value))
        {
            return false;
        }

        var parts = value.Split('/', 2, StringSplitOptions.TrimEntries);
        if (parts.Length != 2
            || !double.TryParse(parts[0], NumberStyles.Float, CultureInfo.InvariantCulture, out var numerator)
            || !double.TryParse(parts[1], NumberStyles.Float, CultureInfo.InvariantCulture, out var denominator)
            || !double.IsFinite(numerator)
            || !double.IsFinite(denominator)
            || numerator <= 0
            || denominator <= 0)
        {
            return false;
        }

        frameRate = numerator / denominator;
        return double.IsFinite(frameRate) && frameRate > 0;
    }

    private static bool IsAttachedPicture(JsonElement stream) =>
        stream.TryGetProperty("disposition", out var disposition)
        && disposition.ValueKind == JsonValueKind.Object
        && TryGetPositiveUInt(disposition, "attached_pic", out _);

    private static uint? ParseSampleRate(JsonElement stream) =>
        TryGetPositiveUInt(stream, "sample_rate", out var value) ? value : null;

    private static ushort? ParseChannelCount(JsonElement stream) =>
        TryGetPositiveUShort(stream, "channels", out var value) ? value : null;

    private static string? ParseCodecName(JsonElement stream) =>
        TryGetCodecName(stream, out var value) ? value : null;

    private static bool TryGetCodecName(JsonElement element, out string value)
    {
        value = string.Empty;
        return TryGetString(element, "codec_name", out value)
            && value.Length <= MaxCodecNameLength;
    }

    private static bool TryGetPositiveUInt(JsonElement element, string name, out uint value)
    {
        value = 0;
        if (!TryGetStringOrNumber(element, name, out var text)
            || !uint.TryParse(text, NumberStyles.None, CultureInfo.InvariantCulture, out value))
        {
            return false;
        }

        return value > 0;
    }

    private static bool TryGetPositiveUShort(JsonElement element, string name, out ushort value)
    {
        value = 0;
        if (!TryGetStringOrNumber(element, name, out var text)
            || !ushort.TryParse(text, NumberStyles.None, CultureInfo.InvariantCulture, out value))
        {
            return false;
        }

        return value > 0;
    }

    private static bool TryGetDouble(JsonElement element, string name, out double value)
    {
        value = 0;
        if (!TryGetStringOrNumber(element, name, out var text))
        {
            return false;
        }

        return double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out value);
    }

    private static bool TryGetStringOrNumber(JsonElement element, string name, out string value)
    {
        value = string.Empty;
        if (!element.TryGetProperty(name, out var property))
        {
            return false;
        }

        value = property.ValueKind switch
        {
            JsonValueKind.String => property.GetString() ?? string.Empty,
            JsonValueKind.Number => property.GetRawText(),
            _ => string.Empty,
        };
        return value.Length > 0;
    }

    private static bool TryGetString(JsonElement element, string name, out string value)
    {
        value = string.Empty;
        return element.TryGetProperty(name, out var property)
            && property.ValueKind == JsonValueKind.String
            && (value = property.GetString() ?? string.Empty).Length > 0;
    }

    private static MediaProbeError InvalidMetadata() => new(
        MediaProbeFailureCode.MalformedProbeOutput,
        "媒体探测元数据无效。",
        Retryable: false);

    private static MediaProbeError UnsupportedStream() => new(
        MediaProbeFailureCode.UnsupportedMediaStream,
        "媒体流类型或必要参数不受支持。",
        Retryable: false);
}
