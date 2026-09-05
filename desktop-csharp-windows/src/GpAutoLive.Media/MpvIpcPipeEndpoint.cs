namespace GpAutoLive.Media;

/// <summary>
/// 受限的 Windows mpv JSON IPC 命名管道端点。
///
/// mpv 在 Windows 上接收的是完整的 <c>\\.\pipe\name</c> 路径，而
/// NamedPipeClientStream 只接收拆分后的 pipe name；此类型同时保存两种形式，
/// 避免调用方自行拼接管道路径。
/// </summary>
public sealed record MpvIpcPipeEndpoint
{
    public const string WindowsPipePrefix = @"\\.\pipe\";
    public const int MaxPipeNameChars = 240;

    private MpvIpcPipeEndpoint(string pipeName)
    {
        PipeName = pipeName;
        PipePath = WindowsPipePrefix + pipeName;
    }

    public string PipeName { get; }

    public string PipePath { get; }

    public static bool TryCreate(
        string? pipePath,
        out MpvIpcPipeEndpoint? endpoint,
        out MpvIpcError? error)
    {
        endpoint = null;
        error = null;

        if (string.IsNullOrWhiteSpace(pipePath)
            || !string.Equals(pipePath, pipePath.Trim(), StringComparison.Ordinal))
        {
            error = Invalid("mpv IPC 命名管道路径不能为空且不能包含首尾空白。");
            return false;
        }

        if (!pipePath.StartsWith(WindowsPipePrefix, StringComparison.OrdinalIgnoreCase))
        {
            error = Invalid("mpv IPC 命名管道必须使用 \\\\.\\pipe\\ 前缀。");
            return false;
        }

        var pipeName = pipePath[WindowsPipePrefix.Length..];
        if (pipeName.Length is 0 or > MaxPipeNameChars
            || pipeName.Contains('\\')
            || pipeName.Contains('/')
            || pipeName.Contains(':')
            || pipeName.Any(character =>
                character is < '!' or > '~'
                || character is '"' or '\'' or '`' or '<' or '>' or '|'))
        {
            error = Invalid("mpv IPC 命名管道名称包含不允许的字符或长度超限。");
            return false;
        }

        endpoint = new MpvIpcPipeEndpoint(pipeName);
        return true;
    }

    public override string ToString() => PipePath;

    private static MpvIpcError Invalid(string message) => new(
        MpvIpcFailureCode.InvalidPipeName,
        message,
        Retryable: false);
}
