namespace GpAutoLive.Windows;

/// <summary>C# 桌面端单实例锁的获取结果。</summary>
public enum WindowsSingleInstanceCode
{
    Acquired,
    AlreadyOwned,
    InvalidPath,
    NotWindows,
    ApiUnavailable,
}

/// <summary>不携带异常正文或用户路径的单实例锁获取结果。</summary>
public sealed record WindowsSingleInstanceResult(
    WindowsSingleInstanceCode Code,
    WindowsSingleInstanceLease? Lease = null)
{
    public bool IsSuccess => Code is WindowsSingleInstanceCode.Acquired && Lease is not null;
}

/// <summary>
/// 以用户本地锁文件持有 C# 桌面端实例。
/// <para>
/// 文件句柄的 <see cref="FileShare.None"/> 是本客户端的进程边界；进程异常退出时由
/// Windows 自动释放句柄，锁文件本身保留为空文件，避免启动清理与新实例之间产生竞态。
/// </para>
/// </summary>
public sealed class WindowsSingleInstanceLease : IDisposable
{
    private readonly FileStream _lockStream;
    private int _disposed;

    private WindowsSingleInstanceLease(FileStream lockStream, string lockPath)
    {
        _lockStream = lockStream;
        LockPath = lockPath;
    }

    /// <summary>实际持有的锁文件路径，仅用于本地诊断。</summary>
    public string LockPath { get; }

    /// <summary>获取默认的用户本地 C# 单实例锁；不会等待已有实例。</summary>
    public static WindowsSingleInstanceResult TryAcquire(string? lockPath = null)
    {
        if (!OperatingSystem.IsWindows())
        {
            return new(WindowsSingleInstanceCode.NotWindows);
        }

        var resolvedPath = ResolveLockPath(lockPath);
        if (resolvedPath is null)
        {
            return new(WindowsSingleInstanceCode.InvalidPath);
        }

        try
        {
            var directory = Path.GetDirectoryName(resolvedPath);
            if (string.IsNullOrWhiteSpace(directory))
            {
                return new(WindowsSingleInstanceCode.InvalidPath);
            }

            Directory.CreateDirectory(directory);
            var stream = new FileStream(
                resolvedPath,
                FileMode.OpenOrCreate,
                FileAccess.ReadWrite,
                FileShare.None,
                bufferSize: 1,
                options: FileOptions.None);
            return new(WindowsSingleInstanceCode.Acquired, new WindowsSingleInstanceLease(stream, resolvedPath));
        }
        catch (IOException exception)
        {
            return new(IsSharingViolation(exception)
                ? WindowsSingleInstanceCode.AlreadyOwned
                : WindowsSingleInstanceCode.ApiUnavailable);
        }
        catch (UnauthorizedAccessException)
        {
            return new(WindowsSingleInstanceCode.ApiUnavailable);
        }
        catch (NotSupportedException)
        {
            return new(WindowsSingleInstanceCode.ApiUnavailable);
        }
        catch (ArgumentException)
        {
            return new(WindowsSingleInstanceCode.InvalidPath);
        }
        catch (System.Security.SecurityException)
        {
            return new(WindowsSingleInstanceCode.ApiUnavailable);
        }
    }

    /// <summary>释放文件句柄；锁文件不删除。</summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
        {
            return;
        }

        _lockStream.Dispose();
    }

    private static string? ResolveLockPath(string? lockPath)
    {
        if (string.IsNullOrWhiteSpace(lockPath))
        {
            var localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
            if (string.IsNullOrWhiteSpace(localAppData))
            {
                return null;
            }

            lockPath = Path.Combine(localAppData, "GpAutoLive", "locks", "csharp-instance.lock");
        }

        if (lockPath.Any(char.IsControl) || !Path.IsPathFullyQualified(lockPath))
        {
            return null;
        }

        try
        {
            return Path.GetFullPath(lockPath);
        }
        catch (ArgumentException)
        {
            return null;
        }
        catch (NotSupportedException)
        {
            return null;
        }
    }

    private static bool IsSharingViolation(IOException exception)
    {
        var win32Error = exception.HResult & 0xFFFF;
        return win32Error is 32 or 33;
    }
}
