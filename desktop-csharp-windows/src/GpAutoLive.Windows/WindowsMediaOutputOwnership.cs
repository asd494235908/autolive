using System.Collections.Concurrent;

namespace GpAutoLive.Windows;

/// <summary>C# 客户端媒体和输出资源所有权门禁结果。</summary>
public enum WindowsMediaOutputOwnershipCode
{
    Acquired,
    AlreadyOwned,
    NotWindows,
    InvalidMutexName,
    ApiUnavailable,
}

/// <summary>媒体和输出资源锁获取结果；不携带进程路径或系统异常正文。</summary>
public sealed record WindowsMediaOutputOwnershipResult(
    WindowsMediaOutputOwnershipCode Code,
    WindowsMediaOutputOwnershipLease? Lease = null)
{
    public bool IsSuccess => Code == WindowsMediaOutputOwnershipCode.Acquired && Lease is not null;
}

/// <summary>
/// C# 客户端持有的媒体/输出资源租约。
/// <para>
/// 该互斥只保护 C# 客户端自己的进程内外输出生命周期；Rust/Tauri 是隔离的
/// 参考客户端，不在 C# 运行时启动、探测或参与此租约。
/// </para>
/// </summary>
public sealed class WindowsMediaOutputOwnershipLease : IDisposable
{
    /// <summary>C# 客户端当前用户会话的媒体/输出资源锁名称。</summary>
    public const string CSharpMutexName = "Local\\GpAutoLive.CSharp.MediaOutput.Owner.v1";

    private static readonly ConcurrentDictionary<string, byte> ProcessOwnedNames = new(StringComparer.OrdinalIgnoreCase);

    private readonly Mutex _mutex;
    private int _disposed;

    private WindowsMediaOutputOwnershipLease(Mutex mutex, string mutexName)
    {
        _mutex = mutex;
        MutexName = mutexName;
    }

    /// <summary>实际持有的受限互斥名称，供诊断和测试核对。</summary>
    public string MutexName { get; }

    /// <summary>尝试立即取得 C# 自己的资源锁；不会等待、启动或探测其他客户端。</summary>
    public static WindowsMediaOutputOwnershipResult TryAcquire(string? mutexName = null)
    {
        if (!OperatingSystem.IsWindows())
        {
            return new(WindowsMediaOutputOwnershipCode.NotWindows);
        }

        mutexName ??= CSharpMutexName;
        if (!IsValidMutexName(mutexName))
        {
            return new(WindowsMediaOutputOwnershipCode.InvalidMutexName);
        }

        if (!ProcessOwnedNames.TryAdd(mutexName, 0))
        {
            return new(WindowsMediaOutputOwnershipCode.AlreadyOwned);
        }

        Mutex? mutex = null;
        var mutexOwned = false;
        var leaseTransferred = false;
        try
        {
            mutex = new Mutex(initiallyOwned: false, mutexName);
            var acquired = false;
            try
            {
                acquired = mutex.WaitOne(millisecondsTimeout: 0);
            }
            catch (AbandonedMutexException)
            {
                // 进程异常退出后的锁已无所有者；立即接管并继续 fail-closed 之外的正常路径。
                acquired = true;
            }

            if (!acquired)
            {
                return new(WindowsMediaOutputOwnershipCode.AlreadyOwned);
            }

            mutexOwned = true;

            var lease = new WindowsMediaOutputOwnershipLease(mutex, mutexName);
            mutex = null;
            mutexOwned = false;
            leaseTransferred = true;
            return new(WindowsMediaOutputOwnershipCode.Acquired, lease);
        }
        catch (UnauthorizedAccessException)
        {
            return new(WindowsMediaOutputOwnershipCode.ApiUnavailable);
        }
        catch (IOException)
        {
            return new(WindowsMediaOutputOwnershipCode.ApiUnavailable);
        }
        catch (NotSupportedException)
        {
            return new(WindowsMediaOutputOwnershipCode.ApiUnavailable);
        }
        catch (WaitHandleCannotBeOpenedException)
        {
            return new(WindowsMediaOutputOwnershipCode.ApiUnavailable);
        }
        catch (System.Security.SecurityException)
        {
            return new(WindowsMediaOutputOwnershipCode.ApiUnavailable);
        }
        catch (ArgumentException)
        {
            return new(WindowsMediaOutputOwnershipCode.ApiUnavailable);
        }
        finally
        {
            if (!leaseTransferred && mutexOwned && mutex is not null)
            {
                TryReleaseMutex(mutex);
            }

            mutex?.Dispose();
            if (!leaseTransferred)
            {
                ProcessOwnedNames.TryRemove(mutexName, out _);
            }
        }
    }

    /// <summary>释放资源锁；重复释放安全，不会删除用户配置或媒体文件。</summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
        {
            return;
        }

        try
        {
            TryReleaseMutex(_mutex);
        }
        finally
        {
            try
            {
                _mutex.Dispose();
            }
            finally
            {
                ProcessOwnedNames.TryRemove(MutexName, out _);
            }
        }
    }

    private static void TryReleaseMutex(Mutex mutex)
    {
        try
        {
            mutex.ReleaseMutex();
        }
        catch (Exception exception) when (
            exception is ApplicationException
            or UnauthorizedAccessException
            or ObjectDisposedException)
        {
            // 进程退出或异常回收时互斥可能已失去所有权；不阻断关闭或回滚。
        }
    }

    private static bool IsValidMutexName(string name) =>
        name.Length is > 0 and <= 260
        && (name.StartsWith("Local\\", StringComparison.Ordinal)
            || name.StartsWith("Global\\", StringComparison.Ordinal))
        && name.All(static character => !char.IsControl(character));

}
