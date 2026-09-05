using System.Text;
using System.Text.Json.Serialization;

namespace GpAutoLive.Contracts;

/// <summary>固定话术跨窗口消息的稳定常量。</summary>
public static class FixedSpeechContractValues
{
    /// <summary>当前固定话术消息版本。</summary>
    public const int Version = 1;

    /// <summary>固定话术命令消息类型。</summary>
    public const string CommandType = "fixed-speech-command";

    /// <summary>固定话术状态消息类型。</summary>
    public const string StatusType = "fixed-speech-status";
}

/// <summary>固定话术输入上限；文本限制按 Unicode code point 计算。</summary>
public static class FixedSpeechInputLimits
{
    /// <summary>操作 ID 最大 UTF-16 长度。</summary>
    public const int MaxOperationIdLength = 128;

    /// <summary>话术文本最大 Unicode 字符数。</summary>
    public const int MaxTextCharacters = 500;

    /// <summary>状态错误摘要最大 UTF-16 长度。</summary>
    public const int MaxErrorLength = 500;
}

/// <summary>固定话术命令动作。</summary>
public enum FixedSpeechAction
{
    /// <summary>开始朗读一段固定文本。</summary>
    Speak,

    /// <summary>取消指定的朗读操作。</summary>
    Cancel
}

/// <summary>固定话术输出状态。</summary>
public enum FixedSpeechStatus
{
    /// <summary>正在等待本地语音能力。</summary>
    Starting,

    /// <summary>本地系统语音正在朗读。</summary>
    Playing,

    /// <summary>朗读正常结束。</summary>
    Completed,

    /// <summary>朗读被用户或更高优先级输入取消。</summary>
    Cancelled,

    /// <summary>朗读失败。</summary>
    Failed
}

/// <summary>固定话术命令；只描述文本，不承载语音或媒体正文。</summary>
public sealed record FixedSpeechCommandDto(
    [property: JsonPropertyName("version")] int Version,
    [property: JsonPropertyName("type")] string Type,
    [property: JsonPropertyName("action")] FixedSpeechAction Action,
    [property: JsonPropertyName("operation_id")] string OperationId,
    [property: JsonPropertyName("text")]
    [property: JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    string? Text = null)
{
    /// <summary>创建严格的开始朗读命令。</summary>
    public static FixedSpeechCommandDto Speak(string operationId, string text) =>
        new(FixedSpeechContractValues.Version, FixedSpeechContractValues.CommandType, FixedSpeechAction.Speak, operationId, text);

    /// <summary>创建严格的取消朗读命令。</summary>
    public static FixedSpeechCommandDto Cancel(string operationId) =>
        new(FixedSpeechContractValues.Version, FixedSpeechContractValues.CommandType, FixedSpeechAction.Cancel, operationId);
}

/// <summary>固定话术状态通知；错误只允许脱敏摘要。</summary>
public sealed record FixedSpeechStatusDto(
    [property: JsonPropertyName("version")] int Version,
    [property: JsonPropertyName("type")] string Type,
    [property: JsonPropertyName("operation_id")] string OperationId,
    [property: JsonPropertyName("status")] FixedSpeechStatus Status,
    [property: JsonPropertyName("error")] string? Error);

/// <summary>固定话术合同的稳定校验错误。</summary>
public sealed record FixedSpeechValidationError(
    string Code,
    string Message);

/// <summary>固定话术合同错误码。</summary>
public static class FixedSpeechErrorCodes
{
    /// <summary>消息为空或整体结构无效。</summary>
    public const string InvalidCommand = "fixed_speech_invalid_command";

    /// <summary>操作 ID 无效。</summary>
    public const string OperationIdInvalid = "fixed_speech_operation_id_invalid";

    /// <summary>话术文本为空。</summary>
    public const string TextRequired = "fixed_speech_text_required";

    /// <summary>话术文本超出 500 字限制。</summary>
    public const string TextTooLong = "fixed_speech_text_too_long";

    /// <summary>状态错误摘要无效。</summary>
    public const string ErrorInvalid = "fixed_speech_error_invalid";
}

/// <summary>固定话术命令和状态的输入边界。</summary>
public static class FixedSpeechContractValidation
{
    /// <summary>校验固定话术命令。</summary>
    public static bool TryValidateCommand(
        FixedSpeechCommandDto? command,
        out FixedSpeechValidationError? error)
    {
        if (command is null
            || command.Version != FixedSpeechContractValues.Version
            || !string.Equals(command.Type, FixedSpeechContractValues.CommandType, StringComparison.Ordinal))
        {
            error = Invalid("固定话术命令版本或类型无效");
            return false;
        }

        if (!TryValidateOperationId(command.OperationId, out error))
        {
            return false;
        }

        return command.Action switch
        {
            FixedSpeechAction.Speak => TryNormalizeText(command.Text, out _, out error),
            FixedSpeechAction.Cancel => Succeed(out error),
            _ => Fail(out error, FixedSpeechErrorCodes.InvalidCommand, "固定话术命令动作无效")
        };
    }

    /// <summary>校验固定话术状态通知。</summary>
    public static bool TryValidateStatus(
        FixedSpeechStatusDto? status,
        out FixedSpeechValidationError? error)
    {
        if (status is null
            || status.Version != FixedSpeechContractValues.Version
            || !string.Equals(status.Type, FixedSpeechContractValues.StatusType, StringComparison.Ordinal))
        {
            error = Invalid("固定话术状态版本或类型无效");
            return false;
        }

        if (!TryValidateOperationId(status.OperationId, out error))
        {
            return false;
        }

        if (!Enum.IsDefined(status.Status))
        {
            error = Invalid("固定话术状态无效");
            return false;
        }

        if (status.Error is not null
            && (status.Error.Length > FixedSpeechInputLimits.MaxErrorLength || status.Error.Contains('\0')))
        {
            error = new(FixedSpeechErrorCodes.ErrorInvalid, "固定话术错误摘要无效");
            return false;
        }

        error = null;
        return true;
    }

    /// <summary>校验操作 ID。</summary>
    public static bool TryValidateOperationId(
        string? operationId,
        out FixedSpeechValidationError? error)
    {
        if (string.IsNullOrWhiteSpace(operationId)
            || operationId.Length > FixedSpeechInputLimits.MaxOperationIdLength
            || operationId.Contains('\0'))
        {
            error = new(FixedSpeechErrorCodes.OperationIdInvalid, "固定话术操作 ID 无效");
            return false;
        }

        error = null;
        return true;
    }

    /// <summary>校验并规范化文本；只裁剪首尾空白，不修改正文。</summary>
    public static bool TryNormalizeText(
        string? text,
        out string normalized,
        out FixedSpeechValidationError? error)
    {
        normalized = text?.Trim() ?? string.Empty;
        if (normalized.Length == 0 || normalized.Contains('\0'))
        {
            error = new(FixedSpeechErrorCodes.TextRequired, "固定话术文本不能为空");
            return false;
        }

        if (normalized.EnumerateRunes().Count() > FixedSpeechInputLimits.MaxTextCharacters)
        {
            error = new(FixedSpeechErrorCodes.TextTooLong, "固定话术文本最多 500 个 Unicode 字符");
            return false;
        }

        error = null;
        return true;
    }

    private static bool Succeed(out FixedSpeechValidationError? error)
    {
        error = null;
        return true;
    }

    private static bool Fail(
        out FixedSpeechValidationError? error,
        string code,
        string message)
    {
        error = new(code, message);
        return false;
    }

    private static FixedSpeechValidationError Invalid(string message) =>
        new(FixedSpeechErrorCodes.InvalidCommand, message);
}
