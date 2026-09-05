using System.Security.Cryptography;

namespace GpAutoLive.Core.Security;

/// <summary>
/// 敏感值的最小边界。实现负责安全存储；调用方不能把返回值写入普通配置或日志。
/// </summary>
public interface ISecretStore
{
    /// <summary>保存指定名称的秘密字节。</summary>
    void Set(string name, ReadOnlySpan<byte> secret);

    /// <summary>尝试读取秘密；调用方负责释放返回缓冲区。</summary>
    bool TryGet(string name, out SecretBuffer secret);

    /// <summary>删除指定名称的秘密。</summary>
    bool Delete(string name);
}

/// <summary>可清零的短期秘密缓冲区，避免把凭据建模成普通 string。</summary>
public sealed class SecretBuffer : IDisposable
{
    private byte[]? _bytes;

    private SecretBuffer(byte[] bytes)
    {
        _bytes = bytes;
    }

    /// <summary>当前秘密字节长度。</summary>
    public int Length => _bytes?.Length ?? 0;

    /// <summary>从字节复制创建秘密缓冲区。</summary>
    public static SecretBuffer FromBytes(ReadOnlySpan<byte> bytes)
    {
        return new SecretBuffer(bytes.ToArray());
    }

    /// <summary>将秘密复制到调用方提供的缓冲区。</summary>
    public void CopyTo(Span<byte> destination)
    {
        var bytes = _bytes ?? throw new ObjectDisposedException(nameof(SecretBuffer));
        if (destination.Length < bytes.Length)
        {
            throw new ArgumentException("目标缓冲区太小。", nameof(destination));
        }

        bytes.AsSpan().CopyTo(destination);
    }

    /// <summary>清零并释放秘密缓冲区。</summary>
    public void Dispose()
    {
        var bytes = Interlocked.Exchange(ref _bytes, null);
        if (bytes is not null)
        {
            CryptographicOperations.ZeroMemory(bytes);
        }

        GC.SuppressFinalize(this);
    }

    /// <summary>最终化时清零秘密字节。</summary>
    ~SecretBuffer()
    {
        Dispose();
    }
}
