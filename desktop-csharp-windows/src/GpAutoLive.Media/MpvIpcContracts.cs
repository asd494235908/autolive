using System.Collections.Immutable;
using System.Text;
using System.Text.Json;
using GpAutoLive.Contracts;

namespace GpAutoLive.Media;

/// <summary>允许发送到 mpv 的有限命令种类。</summary>
public enum MpvIpcCommandKind
{
    LoadFileReplace,
    SetPause,
    SetPlaybackSpeed,
    SeekAbsoluteMs,
    SetShaderOptions,
    RemoveCpu4FilterChain,
    InstallCpu4FilterChain,
    UpdateCpu4Filter,
    GetProperty,
    Quit,
}

/// <summary>mpv 允许读取的属性白名单。</summary>
public enum MpvIpcProperty
{
    VideoOutputConfigured,
    HardwareDecoderCurrent,
    VideoOutputPasses,
    FrameDropCount,
    DecoderFrameDropCount,
    MistimedFrameCount,
    VideoOutputDelayedFrameCount,
    PlaybackTime,
    EstimatedVideoFps,
    EstimatedFrameNumber,
    EofReached,
    Paused,
    Seeking,
    PausedForCache,
    MediaPath,
    ShaderOptions,
    VideoFilterChain,
}

/// <summary>mpv IPC 帧的有限分类。</summary>
public enum MpvIpcFrameKind
{
    Response,
    Event,
}

/// <summary>IPC 解析和命令执行失败的稳定分类。</summary>
public enum MpvIpcFailureCode
{
    InvalidRequestId,
    InvalidCommand,
    CommandTooLarge,
    MalformedJson,
    ResponseNotObject,
    UnknownField,
    MissingResponseField,
    ResponseRequestIdMismatch,
    CommandRejected,
    PropertyUnavailable,
    IpcDisconnected,
    IpcTimeout,
    QueueFull,
    StalePlaybackIdentity,
    InvalidPipeName,
    IpcNotConnected,
    IpcCancelled,
}

/// <summary>
/// 脱敏的 IPC 错误；不保存 mpv 原始错误文本，避免将路径或外部输出传播到 GUI。
/// </summary>
public sealed record MpvIpcError(
    MpvIpcFailureCode Code,
    string Message,
    bool Retryable,
    ulong? RequestId = null,
    MpvIpcCommandKind? CommandKind = null);

/// <summary>解析后的 mpv IPC 帧；Data 已 Clone，可脱离 JsonDocument 生命周期。</summary>
public sealed record MpvIpcFrame(
    MpvIpcFrameKind Kind,
    ulong? RequestId,
    bool? IsSuccess,
    string? EventName,
    JsonElement? Data,
    MpvIpcError? Error);

/// <summary>mpv IPC 解析结果。</summary>
public sealed record MpvIpcFrameParseResult(
    MpvIpcFrame? Frame,
    MpvIpcError? Error)
{
    public bool IsSuccess => Error is null && Frame is not null;

    public static MpvIpcFrameParseResult Succeeded(MpvIpcFrame frame) => new(frame, null);

    public static MpvIpcFrameParseResult Failed(MpvIpcError error) => new(null, error);
}

/// <summary>
/// mpv JSON IPC 的安全命令工厂。调用方只能得到固定命令，不可传入任意命令名、脚本或 shell 字符串。
/// </summary>
public abstract record MpvIpcCommand
{
    public const int MaxJsonLineBytes = 64 * 1024;
    public const double MinPlaybackSpeed = 0.25;
    public const double MaxPlaybackSpeed = 4.0;

    public abstract MpvIpcCommandKind Kind { get; }

    public abstract string Operation { get; }

    // Keep the public type closed to this assembly's fixed command factories. External callers
    // cannot derive a command that injects an arbitrary mpv command because the base constructor
    // is internal.
    internal MpvIpcCommand()
    {
    }

    public static MpvIpcCommand LoadFileReplace(MpvActiveSource source, ulong sourceStartMs) =>
        new LoadFileReplaceCommand(source, sourceStartMs);

    public static MpvIpcCommand SetPause(bool paused) => new SetPauseCommand(paused);

    public static MpvIpcCommand SetPlaybackSpeed(double speed) => new SetPlaybackSpeedCommand(speed);

    public static MpvIpcCommand SeekAbsoluteMs(ulong positionMs) => new SeekAbsoluteMsCommand(positionMs);

    public static MpvIpcCommand SetShaderOptions(MpvShaderOptionsSnapshot options) =>
        new SetShaderOptionsCommand(options);

    public static MpvIpcCommand RemoveCpu4FilterChain() => new RemoveCpu4FilterChainCommand();

    public static MpvIpcCommand InstallCpu4FilterChain() => new InstallCpu4FilterChainCommand(null);

    public static MpvIpcCommand InstallCpu4FilterChain(MpvVideoEffectSnapshot snapshot) =>
        new InstallCpu4FilterChainCommand(snapshot);

    public static MpvIpcCommand UpdateCpu4Filter(MpvCpu4Parameter parameter, double mappedValue) =>
        new UpdateCpu4FilterCommand(parameter, mappedValue);

    public static MpvIpcCommand GetProperty(MpvIpcProperty property) => new GetPropertyCommand(property);

    public static MpvIpcCommand Quit() => new QuitCommand();

    internal bool IsCompatibleWith(MpvActiveSource activeSource) => this switch
    {
        LoadFileReplaceCommand load => load.Source == activeSource,
        _ => true,
    };

    /// <summary>
    /// 生成一行 mpv JSON IPC 请求。返回 false 时不会输出原始参数或错误命令。
    /// </summary>
    public bool TrySerialize(
        ulong requestId,
        out string? jsonLine,
        out MpvIpcError? error)
    {
        jsonLine = null;
        error = null;
        if (requestId == 0)
        {
            error = new(
                MpvIpcFailureCode.InvalidRequestId,
                "IPC request_id 必须大于 0。",
                Retryable: false,
                CommandKind: Kind);
            return false;
        }

        if (!TryBuildArguments(out var arguments, out error))
        {
            return false;
        }

        try
        {
            var envelope = new Dictionary<string, object?>(StringComparer.Ordinal)
            {
                ["command"] = arguments,
                ["request_id"] = requestId,
            };
            jsonLine = JsonSerializer.Serialize(envelope) + "\n";
            if (Encoding.UTF8.GetByteCount(jsonLine) > MaxJsonLineBytes)
            {
                jsonLine = null;
                error = new(
                    MpvIpcFailureCode.CommandTooLarge,
                    "mpv IPC 命令超过大小限制。",
                    Retryable: false,
                    requestId,
                    Kind);
                return false;
            }

            return true;
        }
        catch (Exception exception) when (exception is ArgumentException or InvalidOperationException)
        {
            error = new(
                MpvIpcFailureCode.InvalidCommand,
                "mpv IPC 命令无法序列化。",
                Retryable: false,
                requestId,
                Kind);
            return false;
        }
    }

    protected abstract bool TryBuildArguments(
        out ImmutableArray<object?> arguments,
        out MpvIpcError? error);

    private sealed record LoadFileReplaceCommand(
        MpvActiveSource Source,
        ulong SourceStartMs) : MpvIpcCommand
    {
        public override MpvIpcCommandKind Kind => MpvIpcCommandKind.LoadFileReplace;

        public override string Operation => "loadfile replace";

        protected override bool TryBuildArguments(
            out ImmutableArray<object?> arguments,
            out MpvIpcError? error)
        {
            arguments = [];
            error = null;
            if (Source is null
                || Source.MediaPath.Kind is not MediaKind.Video
                || MediaPathPolicy.Validate(
                    Source.MediaPath.CanonicalPath,
                    requireExistingFile: false,
                    out var validatedPath) is not null
                || validatedPath.Kind is not MediaKind.Video)
            {
                error = InvalidCommand(Kind, "mpv 活动源无效。", Retryable: false);
                return false;
            }

            if (SourceStartMs > 0
                && Source.DurationMs is ulong duration
                && SourceStartMs >= duration)
            {
                error = InvalidCommand(Kind, "mpv 源起始位置超出媒体时长。", Retryable: false);
                return false;
            }

            var seconds = $"{SourceStartMs / 1_000}.{SourceStartMs % 1_000:000}";
            arguments =
            [
                "loadfile",
                validatedPath.CanonicalPath,
                "replace",
                -1,
                new Dictionary<string, object?>(StringComparer.Ordinal) { ["start"] = seconds },
            ];
            return true;
        }
    }

    private sealed record SetPauseCommand(bool Paused) : MpvIpcCommand
    {
        public override MpvIpcCommandKind Kind => MpvIpcCommandKind.SetPause;

        public override string Operation => "set pause";

        protected override bool TryBuildArguments(
            out ImmutableArray<object?> arguments,
            out MpvIpcError? error)
        {
            error = null;
            arguments = ["set_property", "pause", Paused];
            return true;
        }
    }

    private sealed record SetPlaybackSpeedCommand(double Speed) : MpvIpcCommand
    {
        public override MpvIpcCommandKind Kind => MpvIpcCommandKind.SetPlaybackSpeed;

        public override string Operation => "set speed";

        protected override bool TryBuildArguments(
            out ImmutableArray<object?> arguments,
            out MpvIpcError? error)
        {
            arguments = [];
            error = null;
            if (!double.IsFinite(Speed) || Speed < MinPlaybackSpeed || Speed > MaxPlaybackSpeed)
            {
                error = InvalidCommand(Kind, "mpv 播放速度超出 0.25..4.0 范围。", Retryable: false);
                return false;
            }

            arguments = ["set_property", "speed", Speed];
            return true;
        }
    }

    private sealed record SeekAbsoluteMsCommand(ulong PositionMs) : MpvIpcCommand
    {
        public override MpvIpcCommandKind Kind => MpvIpcCommandKind.SeekAbsoluteMs;

        public override string Operation => "seek absolute";

        protected override bool TryBuildArguments(
            out ImmutableArray<object?> arguments,
            out MpvIpcError? error)
        {
            error = null;
            // mpv JSON IPC uses a JSON number in seconds; keep the value within exact f64 integer range.
            if (PositionMs > 9_007_199_254_740_991UL)
            {
                arguments = [];
                error = InvalidCommand(Kind, "mpv seek 位置超出精确时间戳范围。", Retryable: false);
                return false;
            }

            arguments = ["seek", PositionMs / 1_000d, "absolute+exact"];
            return true;
        }
    }

    private sealed record SetShaderOptionsCommand(MpvShaderOptionsSnapshot Options) : MpvIpcCommand
    {
        public override MpvIpcCommandKind Kind => MpvIpcCommandKind.SetShaderOptions;

        public override string Operation => "set glsl-shader-opts";

        protected override bool TryBuildArguments(
            out ImmutableArray<object?> arguments,
            out MpvIpcError? error)
        {
            arguments = [];
            error = null;
            if (Options is null)
            {
                error = InvalidCommand(Kind, "GPU83 参数快照不能为空。", Retryable: false);
                return false;
            }

            arguments = ["set_property", "glsl-shader-opts", Options.ToMpvValue()];
            return true;
        }
    }

    private sealed record RemoveCpu4FilterChainCommand : MpvIpcCommand
    {
        public override MpvIpcCommandKind Kind => MpvIpcCommandKind.RemoveCpu4FilterChain;

        public override string Operation => "remove cpu4 filter";

        protected override bool TryBuildArguments(
            out ImmutableArray<object?> arguments,
            out MpvIpcError? error)
        {
            error = null;
            arguments = ["vf", "remove", "@autolive_cpu4"];
            return true;
        }
    }

    private sealed record InstallCpu4FilterChainCommand(
        MpvVideoEffectSnapshot? Snapshot) : MpvIpcCommand
    {
        public override MpvIpcCommandKind Kind => MpvIpcCommandKind.InstallCpu4FilterChain;

        public override string Operation => "install cpu4 filter";

        protected override bool TryBuildArguments(
            out ImmutableArray<object?> arguments,
            out MpvIpcError? error)
        {
            error = null;
            if (Snapshot is not null
                && (!Snapshot.TryValidate(out _)
                    || Snapshot.Mode is not MpvVideoProcessingMode.Cpu4))
            {
                arguments = [];
                error = InvalidCommand(Kind, "CPU4 视频参数快照无效。", Retryable: false);
                return false;
            }

            arguments =
            [
                "vf",
                "set",
                Snapshot is null
                    ? MpvLaunchPlan.Cpu4FilterChain
                    : MpvLaunchPlan.CreateCpu4FilterChain(Snapshot),
            ];
            return true;
        }
    }

    private sealed record UpdateCpu4FilterCommand(
        MpvCpu4Parameter Parameter,
        double MappedValue) : MpvIpcCommand
    {
        public override MpvIpcCommandKind Kind => MpvIpcCommandKind.UpdateCpu4Filter;

        public override string Operation => "update cpu4 filter";

        protected override bool TryBuildArguments(
            out ImmutableArray<object?> arguments,
            out MpvIpcError? error)
        {
            arguments = [];
            error = null;
            if (!Enum.IsDefined(Parameter)
                || !double.IsFinite(MappedValue)
                || Parameter switch
                {
                    MpvCpu4Parameter.Brightness => MappedValue is < -1 or > 1,
                    MpvCpu4Parameter.Contrast or MpvCpu4Parameter.Saturation => MappedValue is < 0 or > 2,
                    MpvCpu4Parameter.Hue => MappedValue is < -180 or > 180,
                    _ => true,
                })
            {
                error = InvalidCommand(Kind, "CPU4 滤镜参数无效。", Retryable: false);
                return false;
            }

            var (command, target) = Parameter switch
            {
                MpvCpu4Parameter.Brightness => ("brightness", "eq@autolive_cpu4_eq"),
                MpvCpu4Parameter.Contrast => ("contrast", "eq@autolive_cpu4_eq"),
                MpvCpu4Parameter.Saturation => ("saturation", "eq@autolive_cpu4_eq"),
                MpvCpu4Parameter.Hue => ("h", "hue@autolive_cpu4_hue"),
                _ => (string.Empty, string.Empty),
            };
            arguments = ["vf-command", "autolive_cpu4", command, MappedValue, target];
            return true;
        }
    }

    private sealed record GetPropertyCommand(MpvIpcProperty Property) : MpvIpcCommand
    {
        public override MpvIpcCommandKind Kind => MpvIpcCommandKind.GetProperty;

        public override string Operation => $"get {PropertyName(Property)}";

        protected override bool TryBuildArguments(
            out ImmutableArray<object?> arguments,
            out MpvIpcError? error)
        {
            arguments = [];
            error = null;
            if (!Enum.IsDefined(Property))
            {
                error = InvalidCommand(Kind, "mpv 属性不在读取白名单内。", Retryable: false);
                return false;
            }

            arguments = ["get_property", PropertyName(Property)];
            return true;
        }
    }

    private sealed record QuitCommand : MpvIpcCommand
    {
        public override MpvIpcCommandKind Kind => MpvIpcCommandKind.Quit;

        public override string Operation => "quit";

        protected override bool TryBuildArguments(
            out ImmutableArray<object?> arguments,
            out MpvIpcError? error)
        {
            error = null;
            arguments = ["quit"];
            return true;
        }
    }

    private static string PropertyName(MpvIpcProperty property) => property switch
    {
        MpvIpcProperty.VideoOutputConfigured => "vo-configured",
        MpvIpcProperty.HardwareDecoderCurrent => "hwdec-current",
        MpvIpcProperty.VideoOutputPasses => "vo-passes",
        MpvIpcProperty.FrameDropCount => "frame-drop-count",
        MpvIpcProperty.DecoderFrameDropCount => "decoder-frame-drop-count",
        MpvIpcProperty.MistimedFrameCount => "mistimed-frame-count",
        MpvIpcProperty.VideoOutputDelayedFrameCount => "vo-delayed-frame-count",
        MpvIpcProperty.PlaybackTime => "time-pos",
        MpvIpcProperty.EstimatedVideoFps => "estimated-vf-fps",
        MpvIpcProperty.EstimatedFrameNumber => "estimated-frame-number",
        MpvIpcProperty.EofReached => "eof-reached",
        MpvIpcProperty.Paused => "pause",
        MpvIpcProperty.Seeking => "seeking",
        MpvIpcProperty.PausedForCache => "paused-for-cache",
        MpvIpcProperty.MediaPath => "path",
        MpvIpcProperty.ShaderOptions => "glsl-shader-opts",
        MpvIpcProperty.VideoFilterChain => "vf",
        _ => string.Empty,
    };

    private MpvIpcError InvalidCommand(
        MpvIpcCommandKind kind,
        string message,
        bool Retryable,
        ulong? requestId = null) =>
        new(MpvIpcFailureCode.InvalidCommand, message, Retryable, requestId, kind);
}

/// <summary>严格解析单行 mpv JSON IPC 响应或有限事件帧。</summary>
public static class MpvIpcFrameParser
{
    private static readonly ImmutableHashSet<string> ResponseFields =
        ImmutableHashSet.Create(StringComparer.Ordinal, "request_id", "error", "data");

    private static readonly ImmutableHashSet<string> EventFields =
        ImmutableHashSet.Create(
            StringComparer.Ordinal,
            "event",
            "id",
            "error",
            "name",
            "data",
            "reason",
            "playlist_entry_id",
            "file_error",
            "playlist_insert_id",
            "playlist_insert_num_entries",
            "prefix",
            "level",
            "text",
            "args",
            "result",
            "hook_id");

    public static MpvIpcFrameParseResult Parse(
        string? jsonLine,
        ulong expectedRequestId,
        int maxBytes = MpvIpcCommand.MaxJsonLineBytes)
    {
        if (expectedRequestId == 0)
        {
            return MpvIpcFrameParseResult.Failed(new(
                MpvIpcFailureCode.InvalidRequestId,
                "IPC request_id 必须大于 0。",
                Retryable: false));
        }

        if (maxBytes <= 0 || maxBytes > MpvIpcCommand.MaxJsonLineBytes)
        {
            return MpvIpcFrameParseResult.Failed(new(
                MpvIpcFailureCode.CommandTooLarge,
                "mpv IPC 响应大小上限无效。",
                Retryable: false));
        }

        if (string.IsNullOrWhiteSpace(jsonLine)
            || Encoding.UTF8.GetByteCount(jsonLine) > maxBytes)
        {
            return MpvIpcFrameParseResult.Failed(new(
                MpvIpcFailureCode.CommandTooLarge,
                "mpv IPC 响应为空或超过大小限制。",
                Retryable: false));
        }

        try
        {
            using var document = JsonDocument.Parse(jsonLine);
            var root = document.RootElement;
            if (root.ValueKind is not JsonValueKind.Object)
            {
                return MpvIpcFrameParseResult.Failed(new(
                    MpvIpcFailureCode.ResponseNotObject,
                    "mpv IPC 响应必须是对象。",
                    Retryable: false));
            }

            if (root.TryGetProperty("event", out var eventProperty))
            {
                return ParseEvent(root, eventProperty);
            }

            return ParseResponse(root, expectedRequestId);
        }
        catch (JsonException)
        {
            return MpvIpcFrameParseResult.Failed(new(
                MpvIpcFailureCode.MalformedJson,
                "mpv IPC 响应不是有效 JSON。",
                Retryable: false));
        }
    }

    private static MpvIpcFrameParseResult ParseResponse(JsonElement root, ulong expectedRequestId)
    {
        if (!HasOnlyFields(root, ResponseFields))
        {
            return UnknownFieldError();
        }

        if (!root.TryGetProperty("request_id", out var idProperty)
            || idProperty.ValueKind != JsonValueKind.Number
            || !idProperty.TryGetUInt64(out var requestId)
            || requestId == 0)
        {
            return MpvIpcFrameParseResult.Failed(new(
                MpvIpcFailureCode.InvalidRequestId,
                "mpv IPC 响应 request_id 无效。",
                Retryable: false));
        }

        if (requestId != expectedRequestId)
        {
            return MpvIpcFrameParseResult.Failed(new(
                MpvIpcFailureCode.ResponseRequestIdMismatch,
                "mpv IPC 响应与当前请求不匹配。",
                Retryable: false,
                requestId));
        }

        if (!root.TryGetProperty("error", out var errorProperty)
            || errorProperty.ValueKind != JsonValueKind.String)
        {
            return MpvIpcFrameParseResult.Failed(new(
                MpvIpcFailureCode.MissingResponseField,
                "mpv IPC 响应缺少 error 字段。",
                Retryable: false,
                requestId));
        }

        var errorName = errorProperty.GetString();
        if (string.Equals(errorName, "success", StringComparison.Ordinal))
        {
            JsonElement? data = root.TryGetProperty("data", out var dataProperty)
                ? dataProperty.Clone()
                : null;
            return MpvIpcFrameParseResult.Succeeded(new(
                MpvIpcFrameKind.Response,
                requestId,
                IsSuccess: true,
                EventName: null,
                data,
                Error: null));
        }

        var propertyUnavailable = string.Equals(errorName, "property unavailable", StringComparison.OrdinalIgnoreCase)
            || string.Equals(errorName, "property not found", StringComparison.OrdinalIgnoreCase);
        var code = propertyUnavailable
            ? MpvIpcFailureCode.PropertyUnavailable
            : MpvIpcFailureCode.CommandRejected;
        var message = propertyUnavailable
            ? "mpv 属性当前不可用。"
            : "mpv 命令被拒绝。";
        return MpvIpcFrameParseResult.Succeeded(new(
            MpvIpcFrameKind.Response,
            requestId,
            IsSuccess: false,
            EventName: null,
            Data: root.TryGetProperty("data", out var rejectedData) ? rejectedData.Clone() : null,
            Error: new(code, message, Retryable: propertyUnavailable, requestId)));
    }

    private static MpvIpcFrameParseResult ParseEvent(JsonElement root, JsonElement eventProperty)
    {
        if (!HasOnlyFields(root, EventFields)
            || eventProperty.ValueKind != JsonValueKind.String)
        {
            return UnknownFieldError();
        }

        var eventName = eventProperty.GetString();
        if (string.IsNullOrWhiteSpace(eventName) || eventName.Length > 64)
        {
            return MpvIpcFrameParseResult.Failed(new(
                MpvIpcFailureCode.InvalidCommand,
                "mpv IPC 事件名称无效。",
                Retryable: false));
        }

        JsonElement? data = root.TryGetProperty("data", out var dataProperty)
            ? dataProperty.Clone()
            : null;
        return MpvIpcFrameParseResult.Succeeded(new(
            MpvIpcFrameKind.Event,
            RequestId: null,
            IsSuccess: null,
            eventName,
            data,
            Error: null));
    }

    private static bool HasOnlyFields(JsonElement root, ImmutableHashSet<string> allowed) =>
        root.EnumerateObject().All(property => allowed.Contains(property.Name));

    private static MpvIpcFrameParseResult UnknownFieldError() => MpvIpcFrameParseResult.Failed(new(
        MpvIpcFailureCode.UnknownField,
        "mpv IPC 帧包含未知字段。",
        Retryable: false));
}

/// <summary>从成功 IPC 响应读取有限的布尔、数值和字符串属性。</summary>
public static class MpvIpcValueReader
{
    public static bool TryReadBoolean(
        MpvIpcFrame frame,
        out bool value,
        out MpvIpcError? error)
    {
        value = default;
        error = ValidateSuccessResponse(frame);
        if (error is not null)
        {
            return false;
        }

        if (frame.Data is not JsonElement data || data.ValueKind != JsonValueKind.True && data.ValueKind != JsonValueKind.False)
        {
            error = new(MpvIpcFailureCode.InvalidCommand, "mpv 属性响应必须是布尔值。", Retryable: false);
            return false;
        }

        value = data.GetBoolean();
        return true;
    }

    public static bool TryReadFiniteDouble(
        MpvIpcFrame frame,
        out double? value,
        out MpvIpcError? error)
    {
        value = null;
        error = ValidateSuccessResponse(frame);
        if (error is not null)
        {
            return false;
        }

        if (frame.Data is not JsonElement data)
        {
            error = new(MpvIpcFailureCode.MissingResponseField, "mpv 属性响应缺少 data。", Retryable: false);
            return false;
        }

        if (data.ValueKind == JsonValueKind.Null)
        {
            return true;
        }

        if (data.ValueKind != JsonValueKind.Number || !data.TryGetDouble(out var number) || !double.IsFinite(number))
        {
            error = new(MpvIpcFailureCode.InvalidCommand, "mpv 数值属性响应无效。", Retryable: false);
            return false;
        }

        value = number;
        return true;
    }

    public static bool TryReadString(
        MpvIpcFrame frame,
        out string? value,
        out MpvIpcError? error)
    {
        value = null;
        error = ValidateSuccessResponse(frame);
        if (error is not null)
        {
            return false;
        }

        if (frame.Data is not JsonElement data
            || data.ValueKind != JsonValueKind.String
            || string.IsNullOrWhiteSpace(data.GetString()))
        {
            error = new(MpvIpcFailureCode.InvalidCommand, "mpv 字符串属性响应无效。", Retryable: false);
            return false;
        }

        value = data.GetString();
        return true;
    }

    private static MpvIpcError? ValidateSuccessResponse(MpvIpcFrame frame)
    {
        if (frame is null || frame.Kind is not MpvIpcFrameKind.Response || frame.IsSuccess is not true)
        {
            return new(MpvIpcFailureCode.InvalidCommand, "mpv IPC 帧不是成功响应。", Retryable: false);
        }

        return null;
    }
}
