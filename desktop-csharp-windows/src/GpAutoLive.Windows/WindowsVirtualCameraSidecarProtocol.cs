using System.Buffers.Binary;

namespace GpAutoLive.Windows;

/// <summary>AkVirtualCamera sidecar 固定帧传输协议；本类不打开管道或启动进程。</summary>
public static class WindowsVirtualCameraSidecarProtocol
{
    /// <summary>协议版本。</summary>
    public const ushort ProtocolVersion = 1;
    /// <summary>固定帧 magic。</summary>
    public static ReadOnlySpan<byte> FrameMagic => "GPAKVC01"u8;
    /// <summary>固定帧头长度。</summary>
    public const int FrameHeaderBytes = 52;
    /// <summary>固定 YUY2 输出宽度。</summary>
    public const uint OutputWidth = 1280;
    /// <summary>固定 YUY2 输出高度。</summary>
    public const uint OutputHeight = 720;
    /// <summary>固定 payload 字节数。</summary>
    public const int MaxPayloadBytes = checked((int)(OutputWidth * OutputHeight * 2));
    /// <summary>单帧完整传输字节数。</summary>
    public const int EncodedFrameBytes = FrameHeaderBytes + MaxPayloadBytes;
    /// <summary>受控 Named Pipe 名称前缀。</summary>
    public const string PipePrefix = @"\\.\pipe\GpAutoLive-AkVirtualCamera-";

    /// <summary>编码协议失败时的脱敏错误。</summary>
    public sealed record ProtocolError(string Code, string Message);

    /// <summary>协议帧，时间戳单位为 100ns。</summary>
    public sealed record SidecarFrame(
        ulong Generation,
        ulong Sequence,
        long Timestamp100Ns,
        byte[] Payload);

    /// <summary>将 16 字节随机会话令牌映射为固定管道名。</summary>
    public static bool TryCreatePipeName(
        ReadOnlySpan<byte> sessionToken,
        out string? pipeName,
        out ProtocolError? error)
    {
        pipeName = null;
        error = null;
        if (sessionToken.Length != 16 || sessionToken.IsEmpty || IsAllZero(sessionToken))
        {
            error = InvalidPipeToken();
            return false;
        }

        pipeName = string.Create(PipePrefix.Length + 32, sessionToken.ToArray(), static (destination, token) =>
        {
            PipePrefix.AsSpan().CopyTo(destination);
            for (var index = 0; index < token.Length; index++)
            {
                var offset = PipePrefix.Length + index * 2;
                token[index].TryFormat(destination[offset..], out _, "x2");
            }
        });
        return true;
    }

    /// <summary>验证管道名只由固定前缀和非零 16 字节十六进制令牌组成。</summary>
    public static bool TryValidatePipeName(string? pipeName, out ProtocolError? error)
    {
        error = null;
        if (string.IsNullOrWhiteSpace(pipeName)
            || !pipeName.StartsWith(PipePrefix, StringComparison.Ordinal))
        {
            error = InvalidPipeToken();
            return false;
        }

        var suffix = pipeName.AsSpan(PipePrefix.Length);
        if (suffix.Length != 32 || !IsHex(suffix) || suffix.Trim('0').IsEmpty)
        {
            error = InvalidPipeToken();
            return false;
        }

        return true;
    }

    /// <summary>把一帧写入固定大小目标缓冲，不创建编码数组。</summary>
    public static bool TryEncode(
        SidecarFrame? frame,
        Span<byte> destination,
        out int written,
        out ProtocolError? error)
    {
        written = 0;
        error = ValidateFrame(frame);
        if (error is not null)
        {
            return false;
        }

        if (destination.Length < EncodedFrameBytes)
        {
            error = new("sidecar_frame_buffer_too_small", "sidecar 帧目标缓冲区空间不足");
            return false;
        }

        var value = frame!;
        FrameMagic.CopyTo(destination);
        BinaryPrimitives.WriteUInt16LittleEndian(destination[8..], ProtocolVersion);
        BinaryPrimitives.WriteUInt16LittleEndian(destination[10..], FrameHeaderBytes);
        BinaryPrimitives.WriteUInt64LittleEndian(destination[12..], value.Generation);
        BinaryPrimitives.WriteUInt64LittleEndian(destination[20..], value.Sequence);
        BinaryPrimitives.WriteInt64LittleEndian(destination[28..], value.Timestamp100Ns);
        BinaryPrimitives.WriteUInt32LittleEndian(destination[36..], OutputWidth);
        BinaryPrimitives.WriteUInt32LittleEndian(destination[40..], OutputHeight);
        BinaryPrimitives.WriteUInt32LittleEndian(destination[44..], MaxPayloadBytes);
        BinaryPrimitives.WriteUInt32LittleEndian(destination[48..], 0);
        value.Payload.AsSpan().CopyTo(destination[FrameHeaderBytes..]);
        written = EncodedFrameBytes;
        return true;
    }

    /// <summary>从连续输入缓冲解析一帧，并返回已消费字节数；不会按输入声明长度无界扩容。</summary>
    public static bool TryDecode(
        ReadOnlySpan<byte> input,
        out SidecarFrame? frame,
        out int consumed,
        out ProtocolError? error)
    {
        frame = null;
        consumed = 0;
        error = null;
        if (input.Length < FrameHeaderBytes)
        {
            error = new("sidecar_frame_truncated", "sidecar 帧数据不完整");
            return false;
        }

        if (!input[..8].SequenceEqual(FrameMagic))
        {
            error = InvalidFrame("magic 不匹配");
            return false;
        }

        if (BinaryPrimitives.ReadUInt16LittleEndian(input[8..]) != ProtocolVersion)
        {
            error = InvalidFrame("协议版本不支持");
            return false;
        }

        if (BinaryPrimitives.ReadUInt16LittleEndian(input[10..]) != FrameHeaderBytes)
        {
            error = InvalidFrame("header 长度不匹配");
            return false;
        }

        var generation = BinaryPrimitives.ReadUInt64LittleEndian(input[12..]);
        var sequence = BinaryPrimitives.ReadUInt64LittleEndian(input[20..]);
        var timestamp = BinaryPrimitives.ReadInt64LittleEndian(input[28..]);
        var width = BinaryPrimitives.ReadUInt32LittleEndian(input[36..]);
        var height = BinaryPrimitives.ReadUInt32LittleEndian(input[40..]);
        var payloadLength = BinaryPrimitives.ReadUInt32LittleEndian(input[44..]);
        var reserved = BinaryPrimitives.ReadUInt32LittleEndian(input[48..]);
        if (reserved != 0)
        {
            error = InvalidFrame("保留字段必须为 0");
            return false;
        }

        if (width != OutputWidth || height != OutputHeight || payloadLength != MaxPayloadBytes)
        {
            error = InvalidFrame("输出规格或 payload 长度不是固定 1280×720 YUY2");
            return false;
        }

        if (input.Length < EncodedFrameBytes)
        {
            error = new("sidecar_frame_truncated", "sidecar 帧数据不完整");
            return false;
        }

        var candidate = new SidecarFrame(
            generation,
            sequence,
            timestamp,
            input[FrameHeaderBytes..EncodedFrameBytes].ToArray());
        error = ValidateFrame(candidate);
        if (error is not null)
        {
            return false;
        }

        frame = candidate;
        consumed = EncodedFrameBytes;
        return true;
    }

    private static ProtocolError? ValidateFrame(SidecarFrame? frame)
    {
        if (frame is null)
        {
            return InvalidFrame("帧不能为空");
        }

        if (frame.Generation == 0 || frame.Sequence == 0)
        {
            return InvalidFrame("generation 和 sequence 不能为 0");
        }

        if (frame.Timestamp100Ns < 0)
        {
            return InvalidFrame("时间戳不能为负数");
        }

        if (frame.Payload is null || frame.Payload.Length != MaxPayloadBytes)
        {
            return InvalidFrame("YUY2 payload 必须固定为 1280×720×2");
        }

        return null;
    }

    private static bool IsAllZero(ReadOnlySpan<byte> bytes)
    {
        foreach (var value in bytes)
        {
            if (value != 0)
            {
                return false;
            }
        }

        return true;
    }

    private static bool IsHex(ReadOnlySpan<char> value)
    {
        foreach (var character in value)
        {
            if (!Uri.IsHexDigit(character))
            {
                return false;
            }
        }

        return true;
    }

    private static ProtocolError InvalidFrame(string message) =>
        new("sidecar_frame_invalid", $"sidecar 帧无效：{message}");

    private static ProtocolError InvalidPipeToken() =>
        new("sidecar_pipe_token_invalid", "sidecar Named Pipe 会话令牌无效");
}

