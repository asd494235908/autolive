using System.Collections.Immutable;
using System.Globalization;
using GpAutoLive.Contracts;

namespace GpAutoLive.Media;

/// <summary>受管 mpv 的有限启动模式。</summary>
public enum MpvLaunchMode
{
    Gpu83,
    Cpu4,
    Original,
}

/// <summary>mpv 启动参数校验的稳定错误分类。</summary>
public enum MpvLaunchFailureCode
{
    RuntimeUnavailable,
    MissingMpv,
    InvalidMediaPath,
    InvalidHostWindow,
    InvalidPipe,
    InvalidMode,
    MissingGpu83Shader,
    InvalidStartPosition,
    ArgumentsTooLarge,
}

/// <summary>不包含路径、命令行或底层异常正文的启动错误。</summary>
public sealed record MpvLaunchError(
    MpvLaunchFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>
/// 已完成资源、窗口、媒体和 IPC 校验的固定 mpv 启动说明。
/// Windows 进程宿主只消费此对象，不再自行拼接参数；CPU4 滤镜在 IPC
/// 握手后由会话按效果快照安装，避免启动参数和运行时各自维护一套滤镜状态。
/// </summary>
public sealed class MpvLaunchPlan
{
    private MpvLaunchPlan(
        string executablePath,
        ImmutableArray<string> arguments,
        MpvIpcPipeEndpoint ipcEndpoint,
        ValidatedMediaPath mediaPath,
        uint hostWindowId,
        MpvLaunchMode mode,
        ulong sourceStartMs)
    {
        ExecutablePath = executablePath;
        Arguments = arguments;
        IpcEndpoint = ipcEndpoint;
        MediaPath = mediaPath;
        HostWindowId = hostWindowId;
        Mode = mode;
        SourceStartMs = sourceStartMs;
    }

    public string ExecutablePath { get; }

    public ImmutableArray<string> Arguments { get; }

    public MpvIpcPipeEndpoint IpcEndpoint { get; }

    public ValidatedMediaPath MediaPath { get; }

    public uint HostWindowId { get; }

    public MpvLaunchMode Mode { get; }

    public ulong SourceStartMs { get; }

    public const int MaxArgumentCount = 32;
    public const int MaxArgumentCharacters = 32_000;
    public const string Cpu4FilterChain =
        "@autolive_cpu4:lavfi=[eq@autolive_cpu4_eq=brightness=0:contrast=1:saturation=1,hue@autolive_cpu4_hue=h=0:s=1]";

    /// <summary>按已校验 CPU4 快照生成带固定标签的 mpv lavfi 滤镜链。</summary>
    internal static string CreateCpu4FilterChain(MpvVideoEffectSnapshot snapshot)
    {
        if (!snapshot.TryValidate(out _)
            || snapshot.Mode is not MpvVideoProcessingMode.Cpu4)
        {
            throw new ArgumentException("CPU4 视频快照无效。", nameof(snapshot));
        }

        static string Format(double value) =>
            value.ToString("0.###", CultureInfo.InvariantCulture);

        return $"@autolive_cpu4:lavfi=[eq@autolive_cpu4_eq=brightness={Format(snapshot.MappedCpu4Value(MpvCpu4Parameter.Brightness))}:contrast={Format(snapshot.MappedCpu4Value(MpvCpu4Parameter.Contrast))}:saturation={Format(snapshot.MappedCpu4Value(MpvCpu4Parameter.Saturation))},hue@autolive_cpu4_hue=h={Format(snapshot.MappedCpu4Value(MpvCpu4Parameter.Hue))}:s=1]";
    }

    /// <summary>
    /// 从已校验运行时和固定输入创建启动参数；不会启动进程或访问 IPC。
    /// </summary>
    public static bool TryCreate(
        VerifiedMediaRuntime? runtime,
        string? mediaPath,
        ulong hostWindowId,
        MpvIpcPipeEndpoint? ipcEndpoint,
        MpvLaunchMode mode,
        ulong sourceStartMs,
        ulong? durationMs,
        out MpvLaunchPlan? plan,
        out MpvLaunchError? error)
    {
        plan = null;
        error = null;
        if (runtime is null)
        {
            error = new(MpvLaunchFailureCode.RuntimeUnavailable, "mpv 运行资源尚未校验。", true);
            return false;
        }

        if (!runtime.TryGetResource("mpv.exe", out var mpvResource) || mpvResource is null)
        {
            error = new(MpvLaunchFailureCode.MissingMpv, "已验证的媒体运行资源缺少 mpv。", false);
            return false;
        }

        if (hostWindowId is 0 or > uint.MaxValue)
        {
            error = new(MpvLaunchFailureCode.InvalidHostWindow, "mpv 宿主窗口句柄无效。", false);
            return false;
        }

        if (ipcEndpoint is null)
        {
            error = new(MpvLaunchFailureCode.InvalidPipe, "mpv IPC 命名管道无效。", false);
            return false;
        }

        if (!Enum.IsDefined(mode))
        {
            error = new(MpvLaunchFailureCode.InvalidMode, "mpv 视频模式无效。", false);
            return false;
        }

        VerifiedRuntimeResource? gpu83Shader = null;
        if (mode is MpvLaunchMode.Gpu83
            && (!runtime.TryGetResource("gpu83.hook", out gpu83Shader)
                || gpu83Shader is null))
        {
            error = new(
                MpvLaunchFailureCode.MissingGpu83Shader,
                "已验证的媒体运行资源缺少 GPU83 shader。",
                Retryable: false);
            return false;
        }

        var pathError = MediaPathPolicy.Validate(
            mediaPath,
            requireExistingFile: true,
            out var validatedMediaPath);
        if (pathError is not null || validatedMediaPath.Kind is not MediaKind.Video)
        {
            error = new(MpvLaunchFailureCode.InvalidMediaPath, "mpv 活动媒体必须是可读取的视频文件。", false);
            return false;
        }

        if (durationMs is ulong duration && (duration == 0 || sourceStartMs >= duration))
        {
            error = new(MpvLaunchFailureCode.InvalidStartPosition, "mpv 源起始位置超出媒体时长。", false);
            return false;
        }

        var arguments = ImmutableArray.CreateBuilder<string>(MaxArgumentCount);
        arguments.Add("--no-config");
        arguments.Add("--input-default-bindings=no");
        arguments.Add("--input-vo-keyboard=no");
        arguments.Add("--terminal=no");
        arguments.Add("--keep-open=yes");
        arguments.Add("--vo=gpu-next");
        arguments.Add("--audio=no");
        arguments.Add(mode switch
        {
            MpvLaunchMode.Gpu83 => "--hwdec=auto-safe",
            MpvLaunchMode.Cpu4 => "--hwdec=no",
            MpvLaunchMode.Original => "--hwdec=d3d11va",
            _ => string.Empty,
        });
        if (mode is MpvLaunchMode.Gpu83)
        {
            arguments.Add($"--glsl-shaders={gpu83Shader!.AbsolutePath}");
        }
        arguments.Add($"--start={sourceStartMs / 1_000}.{sourceStartMs % 1_000:000}");
        arguments.Add("--pause=yes");
        arguments.Add($"--wid={hostWindowId}");
        arguments.Add($"--input-ipc-server={ipcEndpoint.PipePath}");
        arguments.Add("--");
        arguments.Add(validatedMediaPath.CanonicalPath);

        if (arguments.Count > MaxArgumentCount
            || arguments.Any(argument => argument.Length > MaxArgumentCharacters)
            || arguments.Sum(argument => (long)argument.Length + 1) + mpvResource.AbsolutePath.Length > 32_767)
        {
            error = new(MpvLaunchFailureCode.ArgumentsTooLarge, "mpv 启动参数超过 Windows 命令长度限制。", false);
            return false;
        }

        plan = new MpvLaunchPlan(
            mpvResource.AbsolutePath,
            arguments.ToImmutable(),
            ipcEndpoint,
            validatedMediaPath,
            (uint)hostWindowId,
            mode,
            sourceStartMs);
        return true;
    }
}
