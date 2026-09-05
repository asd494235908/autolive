using System.Collections.Immutable;
using GpAutoLive.Contracts;
using GpAutoLive.Core.Processes;

namespace GpAutoLive.Media;

/// <summary>RTMP FFmpeg 启动计划错误；不携带地址、路径或 FFmpeg 原文。</summary>
public enum RtmpCommandFailureCode
{
    InvalidConfig,
    InvalidFfmpegPath,
    InvalidSource,
    SourceTrackUnavailable,
    InvalidEncoder,
    InvalidVideoEffects,
    ArgumentsTooLarge,
}

/// <summary>RTMP FFmpeg 计划错误。</summary>
public sealed record RtmpCommandError(
    RtmpCommandFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>
/// 已校验的 RTMP FFmpeg 启动计划。实际进程仍由 Windows 受管进程宿主启动，
/// audio pipe 是否连接最终 PCM 由后续音频所有者负责。
/// </summary>
public sealed record RtmpFfmpegLaunchPlan(
    ExternalProcessPlan ProcessPlan,
    bool RequiresFinalPcmInput,
    RtmpOutputConfig Config,
    RtmpSourceIdentity? SourceIdentity,
    string RedactedTargetUrl,
    string Encoder);

/// <summary>构造固定、无 Shell 拼接的 FFmpeg RTMP 参数。</summary>
public static class RtmpFfmpegCommandBuilder
{
    private const int MaxStandardOutputBytes = 4 * 1024;
    private const int MaxStandardErrorBytes = 64 * 1024;
    private static readonly TimeSpan ProcessTimeout = TimeSpan.FromHours(1);

    /// <summary>
    /// 根据已探测媒体和配置构造计划；不启动进程、不访问网络、不写配置。
    /// </summary>
    public static bool TryCreate(
        RtmpOutputConfig? config,
        SourceMediaDto? source,
        string? ffmpegPath,
        string? preferredEncoder,
        RtmpSourceIdentity? sourceIdentity,
        out RtmpFfmpegLaunchPlan? plan,
        out RtmpCommandError? error,
        MpvVideoEffectSnapshot? videoEffects = null)
    {
        plan = null;
        error = null;
        if (!RtmpOutputRules.TryValidate(config, out var configError) || config is null)
        {
            error = new(
                RtmpCommandFailureCode.InvalidConfig,
                configError?.Message ?? "RTMP 配置无效");
            return false;
        }

        if (string.IsNullOrWhiteSpace(ffmpegPath)
            || !Path.IsPathFullyQualified(ffmpegPath)
            || ffmpegPath.Any(char.IsControl))
        {
            error = new(RtmpCommandFailureCode.InvalidFfmpegPath, "FFmpeg 运行资源路径无效");
            return false;
        }

        var pathError = MediaPathPolicy.Validate(
            source?.SourcePath,
            requireExistingFile: true,
            out var validatedSource);
        if (pathError is not null || source is null)
        {
            error = new(RtmpCommandFailureCode.InvalidSource, "RTMP 源媒体不可用");
            return false;
        }

        if (config.VideoEnabled && source.MediaKind is not MediaKind.Video)
        {
            error = new(RtmpCommandFailureCode.SourceTrackUnavailable, "当前源媒体没有可用画面");
            return false;
        }

        string? videoFilter = null;
        if (config.VideoEnabled
            && !TryCreateVideoFilter(videoEffects, out videoFilter, out var videoFilterError))
        {
            error = videoFilterError;
            return false;
        }

        if (config.AudioEnabled && source.MediaKind is not (MediaKind.Video or MediaKind.Audio))
        {
            error = new(RtmpCommandFailureCode.SourceTrackUnavailable, "当前源媒体没有可用声音");
            return false;
        }

        var encoder = ResolveEncoder(config.VideoEnabled, preferredEncoder, out var encoderError);
        if (encoder is null)
        {
            error = encoderError;
            return false;
        }

        var arguments = ImmutableArray.CreateBuilder<string>(RtmpOutputRules.MaxArgumentCount);
        arguments.Add("-hide_banner");
        arguments.Add("-loglevel");
        arguments.Add("error");
        // audio_enabled 时 stdin 是最终 PCM 输入，不能再关闭标准输入。
        if (!config.AudioEnabled)
        {
            arguments.Add("-nostdin");
        }

        var inputIndex = 0;
        if (config.VideoEnabled)
        {
            arguments.Add("-stream_loop");
            arguments.Add("-1");
            if (sourceIdentity?.SourcePositionMs is > 0 and var positionMs)
            {
                arguments.Add("-ss");
                arguments.Add($"{positionMs / 1_000}.{positionMs % 1_000:000}");
            }

            arguments.Add("-re");
            arguments.Add("-i");
            arguments.Add(validatedSource.CanonicalPath);
            inputIndex = 1;
        }

        if (config.AudioEnabled)
        {
            // The process host does not expose stdin yet; the flag documents the
            // final-PCM boundary and prevents accidentally reading source audio twice.
            arguments.Add("-f");
            arguments.Add("f32le");
            arguments.Add("-ar");
            arguments.Add(RtmpOutputRules.AudioSampleRateHz.ToString());
            arguments.Add("-ac");
            arguments.Add("2");
            arguments.Add("-i");
            arguments.Add("pipe:0");
        }

        if (config.VideoEnabled)
        {
            arguments.Add("-map");
            arguments.Add("0:v:0");
            if (videoFilter is not null)
            {
                arguments.Add("-vf");
                arguments.Add(videoFilter);
            }
            AddVideoEncoderArguments(arguments, encoder);
            arguments.Add("-pix_fmt");
            arguments.Add("yuv420p");
            arguments.Add("-r");
            arguments.Add(config.Fps.ToString());
            arguments.Add("-fps_mode");
            arguments.Add("cfr");
            arguments.Add("-s");
            arguments.Add($"{config.Width}x{config.Height}");
            arguments.Add("-b:v");
            arguments.Add($"{config.VideoBitrateKbps}k");
            arguments.Add("-maxrate");
            arguments.Add($"{config.VideoBitrateKbps}k");
            arguments.Add("-bufsize");
            arguments.Add($"{config.VideoBitrateKbps * 2L}k");
            arguments.Add("-g");
            arguments.Add((config.Fps * 2u).ToString());
            arguments.Add("-bf");
            arguments.Add("0");
        }

        if (config.AudioEnabled)
        {
            arguments.Add("-map");
            arguments.Add($"{inputIndex}:a:0");
            arguments.Add("-c:a");
            arguments.Add("aac");
            arguments.Add("-b:a");
            arguments.Add($"{config.AudioBitrateKbps}k");
            arguments.Add("-ar");
            arguments.Add(RtmpOutputRules.AudioSampleRateHz.ToString());
            arguments.Add("-ac");
            arguments.Add("2");
        }

        arguments.Add("-f");
        arguments.Add("flv");
        arguments.Add("-flvflags");
        arguments.Add("no_duration_filesize");
        arguments.Add(config.TargetUrl);

        if (arguments.Count > RtmpOutputRules.MaxArgumentCount
            || arguments.Any(static argument => argument.Length > RtmpOutputRules.MaxArgumentCharacters)
            || arguments.Sum(static argument => (long)argument.Length + 1) + ffmpegPath.Length > 32_767)
        {
            error = new(RtmpCommandFailureCode.ArgumentsTooLarge, "FFmpeg 启动参数超过 Windows 命令长度限制");
            return false;
        }

        plan = new RtmpFfmpegLaunchPlan(
            new ExternalProcessPlan(
                ffmpegPath,
                arguments.ToImmutable(),
                ProcessLaunchPolicy.HiddenNoShellProcessTree,
                ProcessTimeout,
                MaxStandardOutputBytes,
                MaxStandardErrorBytes),
            config.AudioEnabled,
            config,
            sourceIdentity,
            RtmpOutputRules.RedactTargetUrl(config.TargetUrl),
            encoder);
        return true;
    }

    private static bool TryCreateVideoFilter(
        MpvVideoEffectSnapshot? snapshot,
        out string? filter,
        out RtmpCommandError? error)
    {
        filter = null;
        error = null;
        if (snapshot is null || snapshot.Mode is MpvVideoProcessingMode.Original)
        {
            return true;
        }

        if (snapshot.Mode is not MpvVideoProcessingMode.Cpu4
            || !snapshot.TryValidate(out _))
        {
            error = new(
                RtmpCommandFailureCode.InvalidVideoEffects,
                "RTMP 仅支持已校验的 CPU4 视频效果。");
            return false;
        }

        static string Format(double value) =>
            value.ToString("0.###", System.Globalization.CultureInfo.InvariantCulture);

        filter = string.Join(",", [
            $"eq=brightness={Format(snapshot.MappedCpu4Value(MpvCpu4Parameter.Brightness))}:contrast={Format(snapshot.MappedCpu4Value(MpvCpu4Parameter.Contrast))}:saturation={Format(snapshot.MappedCpu4Value(MpvCpu4Parameter.Saturation))}",
            $"hue=h={Format(snapshot.MappedCpu4Value(MpvCpu4Parameter.Hue))}:s=1",
        ]);
        return true;
    }

    private static string? ResolveEncoder(
        bool videoEnabled,
        string? preferredEncoder,
        out RtmpCommandError? error)
    {
        error = null;
        if (!videoEnabled)
        {
            return string.Empty;
        }

        if (string.IsNullOrWhiteSpace(preferredEncoder))
        {
            return RtmpOutputRules.H264EncoderOrder[0];
        }

        if (!RtmpOutputRules.H264EncoderOrder.Contains(preferredEncoder, StringComparer.Ordinal))
        {
            error = new(RtmpCommandFailureCode.InvalidEncoder, "FFmpeg 视频编码器不受支持");
            return null;
        }

        return preferredEncoder;
    }

    private static void AddVideoEncoderArguments(
        ImmutableArray<string>.Builder arguments,
        string encoder)
    {
        arguments.Add("-c:v");
        arguments.Add(encoder);
        switch (encoder)
        {
            case "h264_nvenc":
                arguments.Add("-preset");
                arguments.Add("p4");
                break;
            case "h264_amf":
                arguments.Add("-quality");
                arguments.Add("speed");
                break;
            case "h264_qsv":
                arguments.Add("-preset");
                arguments.Add("veryfast");
                break;
            case "h264_mf":
                arguments.Add("-quality");
                arguments.Add("speed");
                break;
            case "libopenh264":
                arguments.Add("-threads");
                arguments.Add("4");
                break;
        }
    }
}
