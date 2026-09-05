namespace GpAutoLive.Windows;

/// <summary>解析 AkVirtualCamera sidecar 的受限 stdout 状态行。</summary>
public static class WindowsVirtualCameraSidecarStatusParser
{
    private static readonly ReadOnlyMemory<byte> ClientCountPrefix = "GPAKVC_CLIENTS "u8.ToArray();

    /// <summary>允许 sidecar 报告的最大下游客户端数量。</summary>
    public const uint MaxClientCount = 1024;

    /// <summary>
    /// 只接受 ASCII 形式的 <c>GPAKVC_CLIENTS N</c>，拒绝符号、空格、溢出和附加字段。
    /// 输入不包含换行也可以；末尾的 CR 会被忽略。
    /// </summary>
    public static bool TryParseClientCount(ReadOnlySpan<byte> line, out uint count)
    {
        count = 0;
        var prefix = ClientCountPrefix.Span;
        if (!line.StartsWith(prefix))
        {
            return false;
        }

        var value = line[prefix.Length..];
        if (!value.IsEmpty && value[^1] == (byte)'\r')
        {
            value = value[..^1];
        }

        if (value.IsEmpty)
        {
            return false;
        }

        uint parsed = 0;
        foreach (var digit in value)
        {
            if (digit is < (byte)'0' or > (byte)'9')
            {
                return false;
            }

            var next = parsed * 10U + (uint)(digit - (byte)'0');
            if (next < parsed || next > MaxClientCount)
            {
                return false;
            }

            parsed = next;
        }

        count = parsed;
        return true;
    }
}
