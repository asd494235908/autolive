using System.Collections.Immutable;
using GpAutoLive.Contracts;

namespace GpAutoLive.Media;

/// <summary>FFmpeg PCM 解码计划的稳定错误分类。</summary>
public enum FfmpegPcmDecodeFailureCode
{
    InvalidFfmpegPath,
    FfmpegMissing,
    InvalidSource,
    SourceTrackUnavailable,
    InvalidSampleRate,
    InvalidChannels,
    InvalidAudioEffects,
    InvalidStartPosition,
    ArgumentsTooLarge,
}

/// <summary>不携带路径、命令或 FFmpeg 原文的 PCM 解码计划错误。</summary>
public sealed record FfmpegPcmDecodeError(
    FfmpegPcmDecodeFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>已验证的 FFmpeg f32le 音频解码计划；输出固定为交错 PCM。</summary>
public sealed record FfmpegPcmDecodePlan(
    string ExecutablePath,
    ImmutableArray<string> Arguments,
    string SourcePath,
    int SampleRateHz,
    int Channels,
    TimeSpan Timeout)
{
    /// <summary>音频仅从当前视频时间点恢复时使用；普通首播为 0。</summary>
    public ulong SourceStartMs { get; init; }
}

/// <summary>构造受限的普通声音/插话 PCM 解码计划，不启动进程。</summary>
public static class FfmpegPcmDecodePlanBuilder
{
    public const int MaxArgumentCount = 32;
    public const int MaxArgumentCharacters = 32_000;
    public const int MaxCommandCharacters = 32_767;
    public const int DefaultSampleRateHz = 48_000;
    public const int DefaultChannels = 2;
    public static TimeSpan DefaultTimeout { get; } = TimeSpan.FromHours(1);

    public static bool TryCreate(
        string? ffmpegPath,
        SourceMediaDto? source,
        int sampleRateHz,
        int channels,
        out FfmpegPcmDecodePlan? plan,
        out FfmpegPcmDecodeError? error,
        AudioEffectParams? audioEffects = null,
        ulong sourceStartMs = 0)
    {
        plan = null;
        error = null;
        if (string.IsNullOrWhiteSpace(ffmpegPath)
            || ffmpegPath.Any(char.IsControl)
            || !Path.IsPathFullyQualified(ffmpegPath))
        {
            error = new(FfmpegPcmDecodeFailureCode.InvalidFfmpegPath, "FFmpeg 运行资源路径无效。");
            return false;
        }

        if (!File.Exists(ffmpegPath))
        {
            error = new(FfmpegPcmDecodeFailureCode.FfmpegMissing, "FFmpeg 运行资源不存在。", Retryable: true);
            return false;
        }

        var sourceError = MediaPathPolicy.Validate(
            source?.SourcePath,
            requireExistingFile: true,
            out var validatedSource);
        if (sourceError is not null || source is null)
        {
            error = new(FfmpegPcmDecodeFailureCode.InvalidSource, "PCM 解码源媒体不可用。", sourceError?.Retryable == true);
            return false;
        }

        if (source.MediaKind is not (MediaKind.Audio or MediaKind.Video)
            || string.IsNullOrWhiteSpace(source.AudioCodecName))
        {
            error = new(FfmpegPcmDecodeFailureCode.SourceTrackUnavailable, "源媒体没有可用声音轨道。");
            return false;
        }

        if (sampleRateHz is not (44_100 or 48_000))
        {
            error = new(FfmpegPcmDecodeFailureCode.InvalidSampleRate, "PCM 解码采样率仅支持 44100 或 48000 Hz。");
            return false;
        }

        if (channels is < 1 or > 2)
        {
            error = new(FfmpegPcmDecodeFailureCode.InvalidChannels, "PCM 解码声道数必须在 1 到 2 之间。");
            return false;
        }

        if (sourceStartMs > 0
            && source.DurationMs is ulong durationMs
            && (durationMs == 0 || sourceStartMs >= durationMs))
        {
            error = new(FfmpegPcmDecodeFailureCode.InvalidStartPosition, "PCM 解码起始位置超出媒体时长。");
            return false;
        }

        if (audioEffects is not null
            && !audioEffects.TryValidate(out var effectErrors))
        {
            error = new(
                FfmpegPcmDecodeFailureCode.InvalidAudioEffects,
                effectErrors.FirstOrDefault()?.Message ?? "声音效果参数无效。",
                Retryable: false);
            return false;
        }

        if (audioEffects is not null
            && !TryValidateRealtimeConsumption(audioEffects, source.DurationMs, sampleRateHz, out var consumptionError))
        {
            error = new(FfmpegPcmDecodeFailureCode.InvalidAudioEffects, consumptionError);
            return false;
        }

        var argumentList = new List<string>
        {
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-re",
        };
        if (sourceStartMs > 0)
        {
            argumentList.Add("-ss");
            argumentList.Add(FormatPosition(sourceStartMs));
        }

        argumentList.AddRange(
        [
            "-i",
            validatedSource.CanonicalPath,
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-dn",
        ]);
        if (audioEffects is not null)
        {
            var filter = FfmpegAudioFilterBuilder.Create(audioEffects, source.DurationMs, sampleRateHz);
            if (!string.IsNullOrEmpty(filter))
            {
                argumentList.Add("-af");
                argumentList.Add(filter);
            }
        }

        argumentList.AddRange(
        [
            "-f",
            "f32le",
            "-ar",
            sampleRateHz.ToString(System.Globalization.CultureInfo.InvariantCulture),
            "-ac",
            channels.ToString(System.Globalization.CultureInfo.InvariantCulture),
            "pipe:1",
        ]);

        var arguments = argumentList.ToImmutableArray();

        if (arguments.Length > MaxArgumentCount
            || arguments.Any(static argument => argument.Length > MaxArgumentCharacters)
            || arguments.Sum(static argument => (long)argument.Length + 1) + ffmpegPath.Length > MaxCommandCharacters)
        {
            error = new(FfmpegPcmDecodeFailureCode.ArgumentsTooLarge, "PCM 解码启动参数超过 Windows 命令长度限制。");
            return false;
        }

        plan = new(
            ffmpegPath,
            arguments,
            validatedSource.CanonicalPath,
            sampleRateHz,
            channels,
            DefaultTimeout)
        {
            SourceStartMs = sourceStartMs,
        };
        return true;
    }

    private static string FormatPosition(ulong milliseconds) =>
        $"{milliseconds / 1_000}.{milliseconds % 1_000:000}";

    private static bool TryValidateRealtimeConsumption(
        AudioEffectParams parameters,
        ulong? sourceDurationMs,
        int sampleRateHz,
        out string errorMessage)
    {
        if (parameters.NaturalVoiceMode is not (NaturalVoiceMode.Original or NaturalVoiceMode.NaturalDynamic))
        {
            errorMessage = "自然声音模式没有可用的实时消费者。";
            return false;
        }

        if (parameters.EnvironmentNoisePercent != 0
            || parameters.EnvironmentNoiseDbfs != -40
            || parameters.MfccShiftPercent != 0
            || parameters.AmbientSoundMixPercent != 0
            || parameters.DryWetPercent != 0
            || parameters.MfccDimensions != 13
            || parameters.SnrVariationDb != 0
            || parameters.FormantShiftPercent != 0
            || parameters.SnrTargetDb is not null
            || parameters.CurrentFormantHz is not null
            || parameters.OutputBitrateKbps != 192)
        {
            errorMessage = "声音参数包含尚未接入 FFmpeg 实时 PCM 消费者的字段。";
            return false;
        }

        if (parameters.FadeOutMs > 0
            && sourceDurationMs is not (> 0))
        {
            errorMessage = "淡出参数需要可靠的源媒体时长才能接入实时 PCM。";
            return false;
        }

        if (parameters.SampleRateHz is uint targetSampleRate
            && targetSampleRate != (uint)sampleRateHz)
        {
            errorMessage = "目标采样率与 PCM 解码计划不一致。";
            return false;
        }

        errorMessage = string.Empty;
        return true;
    }
}
