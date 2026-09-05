using System.Diagnostics;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace GpAutoLive.Windows;

/// <summary>
/// Windows Job Object 的最小生命周期边界。创建失败或当前系统不可用时由调用方安全回退，
/// 不向上暴露 Win32 错误细节。
/// </summary>
public sealed class WindowsJobObject : IDisposable
{
    private const uint KillProcessTreeOnClose = 0x0000_2000;
    private const int ExtendedLimitInformation = 9;
    private const uint DefaultTerminateExitCode = 0xC000_013A;

    private readonly object _gate = new();
    private SafeFileHandle? _handle;
    private bool _disposed;

    private WindowsJobObject(SafeFileHandle handle) => _handle = handle;

    /// <summary>尝试创建一个关闭时终止其进程树的 Job Object。</summary>
    public static bool TryCreate(out WindowsJobObject? job)
    {
        job = null;
        if (!OperatingSystem.IsWindows())
        {
            return false;
        }

        SafeFileHandle? handle = null;
        try
        {
            var nativeHandle = CreateJobObjectW(IntPtr.Zero, null);
            if (nativeHandle == IntPtr.Zero)
            {
                return false;
            }

            handle = new SafeFileHandle(nativeHandle, ownsHandle: true);
            var limits = new JobObjectExtendedLimitInformation
            {
                BasicLimitInformation = new JobObjectBasicLimitInformation
                {
                    LimitFlags = KillProcessTreeOnClose,
                },
            };

            if (!SetInformationJobObject(
                    handle,
                    ExtendedLimitInformation,
                    ref limits,
                    Marshal.SizeOf<JobObjectExtendedLimitInformation>()))
            {
                handle.Dispose();
                return false;
            }

            job = new WindowsJobObject(handle);
            handle = null;
            return true;
        }
        catch (DllNotFoundException)
        {
            return false;
        }
        catch (EntryPointNotFoundException)
        {
            return false;
        }
        catch (BadImageFormatException)
        {
            return false;
        }
        catch (UnauthorizedAccessException)
        {
            return false;
        }
        catch (System.Security.SecurityException)
        {
            return false;
        }
        finally
        {
            handle?.Dispose();
        }
    }

    /// <summary>把已启动的进程加入 Job Object；失败时保持调用方可选择的普通清理路径。</summary>
    public bool TryAssign(Process process)
    {
        ArgumentNullException.ThrowIfNull(process);

        lock (_gate)
        {
            if (_disposed || _handle is null || _handle.IsInvalid)
            {
                return false;
            }

            try
            {
                return AssignProcessToJobObject(_handle, process.Handle);
            }
            catch (InvalidOperationException)
            {
                return false;
            }
            catch (System.ComponentModel.Win32Exception)
            {
                return false;
            }
            catch (NotSupportedException)
            {
                return false;
            }
            catch (ArgumentException)
            {
                return false;
            }
        }
    }

    /// <summary>显式终止 Job Object 中的进程树。</summary>
    public bool TryTerminate(uint exitCode = DefaultTerminateExitCode)
    {
        lock (_gate)
        {
            if (_disposed || _handle is null || _handle.IsInvalid)
            {
                return false;
            }

            try
            {
                return TerminateJobObject(_handle, exitCode);
            }
            catch (System.ComponentModel.Win32Exception)
            {
                return false;
            }
            catch (ObjectDisposedException)
            {
                return false;
            }
        }
    }

    public void Dispose()
    {
        SafeFileHandle? handle;
        lock (_gate)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            handle = _handle;
            _handle = null;
        }

        handle?.Dispose();
        GC.SuppressFinalize(this);
    }

    [DllImport("kernel32.dll", EntryPoint = "CreateJobObjectW", ExactSpelling = true, SetLastError = true)]
    private static extern IntPtr CreateJobObjectW(IntPtr jobAttributes, string? name);

    [DllImport("kernel32.dll", ExactSpelling = true, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetInformationJobObject(
        SafeFileHandle job,
        int informationClass,
        ref JobObjectExtendedLimitInformation information,
        int informationLength);

    [DllImport("kernel32.dll", ExactSpelling = true, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AssignProcessToJobObject(SafeFileHandle job, IntPtr process);

    [DllImport("kernel32.dll", ExactSpelling = true, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool TerminateJobObject(SafeFileHandle job, uint exitCode);

    [StructLayout(LayoutKind.Sequential)]
    private struct JobObjectExtendedLimitInformation
    {
        public JobObjectBasicLimitInformation BasicLimitInformation;
        public IoCounters IoInfo;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct JobObjectBasicLimitInformation
    {
        public long PerProcessUserTimeLimit;
        public long PerJobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass;
        public uint SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct IoCounters
    {
        public ulong ReadOperationCount;
        public ulong WriteOperationCount;
        public ulong OtherOperationCount;
        public ulong ReadTransferCount;
        public ulong WriteTransferCount;
        public ulong OtherTransferCount;
    }
}
