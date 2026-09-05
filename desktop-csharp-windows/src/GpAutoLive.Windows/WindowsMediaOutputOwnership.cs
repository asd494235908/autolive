using System.Diagnostics;
using System.ComponentModel;
using System.Collections.Concurrent;

namespace GpAutoLive.Windows;

/// <summary>媒体和输出资源跨客户端所有权门禁结果。</summary>
public enum WindowsMediaOutputOwnershipCode
{
    Acquired,
    AlreadyOwned,
    ReferenceClientRunning,
    ReferenceClientProbeFailed,
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
/// C# 客户端持有的全局媒体/输出资源租约。
/// <para>
/// 互斥名称是 C# 与 Rust/Tauri 共同遵守的跨客户端协议；进程探测仅用于
/// 提供更明确的启动提示，命名 Mutex 才是实际的双边资源所有权门禁。
/// </para>
/// </summary>
public sealed class WindowsMediaOutputOwnershipLease : IDisposable
{
    /// <summary>预留给所有桌面客户端的同一用户会话资源锁名称。</summary>
    public const string SharedMutexName = "Local\\GpAutoLive.MediaOutput.Owner.v1";

    /// <summary>现有 Rust/Tauri 正式桌面进程的文件名（不含 .exe）。</summary>
    public const string ReferenceClientProcessName = "autolive-desktop-core";

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

    /// <summary>
    /// 尝试立即取得资源锁。不会等待其他客户端，不会启动或终止任何进程。
    /// <paramref name="referenceClientProbe"/> 仅用于测试或宿主注入；生产默认探测 Rust 进程，
    /// 但最终仍以共享命名 Mutex 的立即获取结果为准。
    /// </summary>
    public static WindowsMediaOutputOwnershipResult TryAcquire(
        string? mutexName = null,
        Func<bool>? referenceClientProbe = null)
    {
        if (!OperatingSystem.IsWindows())
        {
            return new(WindowsMediaOutputOwnershipCode.NotWindows);
        }

        mutexName ??= SharedMutexName;
        if (!IsValidMutexName(mutexName))
        {
            return new(WindowsMediaOutputOwnershipCode.InvalidMutexName);
        }

        var initialReferenceProbe = TryProbeReferenceClient(referenceClientProbe);
        if (initialReferenceProbe is null)
        {
            return new(WindowsMediaOutputOwnershipCode.ReferenceClientProbeFailed);
        }

        if (initialReferenceProbe.Value)
        {
            return new(WindowsMediaOutputOwnershipCode.ReferenceClientRunning);
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

            // The first process probe and mutex acquisition are separate kernel operations.
            // Re-check while holding the mutex so a reference client that starts in that
            // window cannot be accepted by this client. The Rust/Tauri client now consumes
            // this same named mutex; the process probe remains only a diagnostic fast path.
            var finalReferenceProbe = TryProbeReferenceClient(referenceClientProbe);
            if (finalReferenceProbe is null)
            {
                return new(WindowsMediaOutputOwnershipCode.ReferenceClientProbeFailed);
            }

            if (finalReferenceProbe.Value)
            {
                return new(WindowsMediaOutputOwnershipCode.ReferenceClientRunning);
            }

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

    private static bool? TryProbeReferenceClient(Func<bool>? referenceClientProbe)
    {
        if (referenceClientProbe is null)
        {
            return TryDetectReferenceClient();
        }

        try
        {
            return referenceClientProbe();
        }
        catch (Exception exception) when (IsProbeFailure(exception))
        {
            return null;
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

    private static bool? TryDetectReferenceClient()
    {
        try
        {
            foreach (var process in Process.GetProcessesByName(ReferenceClientProcessName))
            {
                try
                {
                    if (!process.HasExited)
                    {
                        return true;
                    }
                }
                catch (InvalidOperationException)
                {
                    // 进程在枚举期间退出；继续检查其他实例。
                }
                finally
                {
                    process.Dispose();
                }
            }

            return false;
        }
        catch (Exception exception) when (IsProbeFailure(exception))
        {
            return null;
        }
    }

    private static bool IsProbeFailure(Exception exception) =>
        exception is InvalidOperationException
            or UnauthorizedAccessException
            or NotSupportedException
            or PlatformNotSupportedException
            or Win32Exception;
}
