using System.Collections.Immutable;
using System.Runtime.InteropServices;

namespace GpAutoLive.Windows;

/// <summary>PortAudio 设备探测的稳定失败分类。</summary>
public enum WindowsPortAudioFailureCode
{
    /// <summary>当前平台不是 Windows。</summary>
    NotWindows,
    /// <summary>DLL 路径不是受支持的绝对路径。</summary>
    InvalidPath,
    /// <summary>PortAudio DLL 不存在。</summary>
    ResourceMissing,
    /// <summary>PortAudio DLL 无法加载或缺少必要导出。</summary>
    NativeLoadFailed,
    /// <summary>PortAudio 初始化失败。</summary>
    InitializeFailed,
    /// <summary>PortAudio 设备查询失败。</summary>
    DeviceQueryFailed,
    /// <summary>调用方取消探测。</summary>
    Cancelled,
    /// <summary>枚举器已关闭。</summary>
    Closed,
}

/// <summary>不会回显 DLL 路径或原生异常正文的 PortAudio 错误。</summary>
public sealed record WindowsPortAudioError(
    WindowsPortAudioFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>一个经过长度和控制字符清理的本地音频设备快照。</summary>
public sealed record WindowsPortAudioDevice(
    int Index,
    string Name,
    string HostApi,
    int MaxInputChannels,
    int MaxOutputChannels,
    double DefaultSampleRate);

/// <summary>PortAudio 设备枚举的脱敏状态。</summary>
public sealed record WindowsPortAudioSnapshot(
    bool IsAvailable,
    string Backend,
    ImmutableArray<WindowsPortAudioDevice> Devices,
    int? DefaultOutputDevice,
    string? ErrorCode,
    string? Error);

/// <summary>PortAudio 设备枚举结果。</summary>
public sealed record WindowsPortAudioProbeResult(
    bool IsSuccess,
    WindowsPortAudioSnapshot Snapshot,
    WindowsPortAudioError? Error = null);

/// <summary>
/// Windows x64 PortAudio 设备枚举边界。DLL 只从调用方提供的固定资源路径动态加载，
/// 不搜索 PATH，不创建长期音频流；真正播放流由后续唯一音频所有者接入。
/// </summary>
public sealed class WindowsPortAudioDeviceEnumerator : IDisposable
{
    internal const string ExpectedDllName = "portaudio_x64.dll";
    private const int MaxDeviceCount = 128;
    private const int MaxTextCharacters = 256;
    private static readonly ImmutableArray<WindowsPortAudioDevice> EmptyDevices = [];
    private readonly object _gate = new();
    private readonly SemaphoreSlim _serial = new(1, 1);
    private bool _disposed;

    /// <summary>读取最近一次脱敏状态。</summary>
    public WindowsPortAudioSnapshot Snapshot { get; private set; } = new(
        IsAvailable: false,
        Backend: "portaudio",
        Devices: EmptyDevices,
        DefaultOutputDevice: null,
        ErrorCode: null,
        Error: null);

    /// <summary>在后台线程完成一次有界设备枚举；取消不会遗留无人管理的原生任务。</summary>
    public async Task<WindowsPortAudioProbeResult> ProbeAsync(
        string? dllPath,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsPortAudioFailureCode.Cancelled, "PortAudio 设备探测已取消。", retryable: true);
        }

        try
        {
            await _serial.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsPortAudioFailureCode.Cancelled, "PortAudio 设备探测已取消。", retryable: true);
        }

        try
        {
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioFailureCode.Closed, "PortAudio 设备枚举器已关闭。", retryable: false);
                }
            }

            // NativeLibrary.Load/Pa_Initialize are synchronous. The task is always awaited,
            // so cancellation is observed at the boundary without abandoning a native handle.
            var probeTask = Task.Run(() => ProbeCore(dllPath), CancellationToken.None);
            WindowsPortAudioProbeResult result;
            try
            {
                result = await probeTask.ConfigureAwait(false);
            }
            catch (Exception exception) when (exception is InvalidOperationException or ObjectDisposedException)
            {
                result = Failure(WindowsPortAudioFailureCode.NativeLoadFailed, "PortAudio 设备探测不可用。", retryable: true);
            }

            if (cancellationToken.IsCancellationRequested)
            {
                return Failure(WindowsPortAudioFailureCode.Cancelled, "PortAudio 设备探测已取消。", retryable: true);
            }

            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsPortAudioFailureCode.Closed, "PortAudio 设备枚举器已关闭。", retryable: false);
                }

                Snapshot = result.Snapshot;
            }

            return result;
        }
        finally
        {
            _serial.Release();
        }
    }

    /// <summary>关闭枚举器；本类不持有长期 PortAudio stream。</summary>
    public void Dispose()
    {
        lock (_gate)
        {
            _disposed = true;
        }

        GC.SuppressFinalize(this);
    }

    private static WindowsPortAudioProbeResult ProbeCore(string? dllPath)
    {
        if (!OperatingSystem.IsWindows())
        {
            return Failure(WindowsPortAudioFailureCode.NotWindows, "PortAudio 设备枚举只支持 Windows。", retryable: false);
        }

        if (!TryValidatePath(dllPath, out var pathError))
        {
            return Failure(
                pathError?.Code ?? WindowsPortAudioFailureCode.InvalidPath,
                pathError?.Message ?? "PortAudio 资源路径无效。",
                pathError?.Retryable ?? false);
        }

        if (!File.Exists(dllPath))
        {
            return Failure(WindowsPortAudioFailureCode.ResourceMissing, "PortAudio 运行资源不存在。", retryable: true);
        }

        WindowsPortAudioNative? native = null;
        try
        {
            if (!WindowsPortAudioNative.TryLoad(dllPath!, out native) || native is null)
            {
                return Failure(WindowsPortAudioFailureCode.NativeLoadFailed, "PortAudio 运行资源无法加载。", retryable: true);
            }

            var initializeError = native.Initialize();
            if (initializeError != 0)
            {
                return Failure(WindowsPortAudioFailureCode.InitializeFailed, "PortAudio 初始化失败。", retryable: true);
            }

            try
            {
                var deviceCount = native.GetDeviceCount();
                if (deviceCount < 0 || deviceCount > MaxDeviceCount)
                {
                    return Failure(WindowsPortAudioFailureCode.DeviceQueryFailed, "PortAudio 设备数量不可用。", retryable: true);
                }

                var devices = ImmutableArray.CreateBuilder<WindowsPortAudioDevice>(deviceCount);
                for (var index = 0; index < deviceCount; index++)
                {
                    var devicePointer = native.GetDeviceInfo(index);
                    if (devicePointer == 0)
                    {
                        return Failure(WindowsPortAudioFailureCode.DeviceQueryFailed, "PortAudio 设备信息不可用。", retryable: true);
                    }

                    var device = Marshal.PtrToStructure<WindowsPortAudioNative.DeviceInfo>(devicePointer);
                    var hostApi = native.GetHostApiInfo(device.HostApi);
                    devices.Add(new(
                        index,
                        SanitizeNativeText(device.Name),
                        SanitizeNativeText(hostApi.Name),
                        Math.Clamp(device.MaxInputChannels, 0, 128),
                        Math.Clamp(device.MaxOutputChannels, 0, 128),
                        SanitizeSampleRate(device.DefaultSampleRate)));
                }

                var defaultOutputDevice = native.GetDefaultOutputDevice();
                return Success(new(
                    IsAvailable: true,
                    Backend: "portaudio",
                    Devices: devices.ToImmutable(),
                    DefaultOutputDevice: defaultOutputDevice >= 0 ? defaultOutputDevice : null,
                    ErrorCode: null,
                    Error: null));
            }
            finally
            {
                native.Terminate();
            }
        }
        catch (Exception exception) when (exception is AccessViolationException
            or DllNotFoundException
            or EntryPointNotFoundException
            or MarshalDirectiveException
            or SEHException
            or InvalidOperationException)
        {
            return Failure(WindowsPortAudioFailureCode.DeviceQueryFailed, "PortAudio 设备探测失败。", retryable: true);
        }
        finally
        {
            native?.Dispose();
        }
    }

    internal static bool TryValidatePath(string? path, out WindowsPortAudioError? error)
    {
        error = null;
        if (string.IsNullOrWhiteSpace(path)
            || path.Length > 32_000
            || path.Any(char.IsControl)
            || !Path.IsPathFullyQualified(path))
        {
            error = new(WindowsPortAudioFailureCode.InvalidPath, "PortAudio 资源路径必须是有效的绝对路径。", false);
            return false;
        }

        try
        {
            var fullPath = Path.GetFullPath(path);
            if (!string.Equals(Path.GetFileName(fullPath), ExpectedDllName, StringComparison.OrdinalIgnoreCase))
            {
                error = new(WindowsPortAudioFailureCode.InvalidPath, "PortAudio 资源名称不受支持。", false);
                return false;
            }

            if (File.Exists(fullPath) && File.GetAttributes(fullPath).HasFlag(FileAttributes.ReparsePoint))
            {
                error = new(WindowsPortAudioFailureCode.InvalidPath, "PortAudio 资源不能通过重解析点加载。", false);
                return false;
            }
        }
        catch (Exception exception) when (exception is ArgumentException or NotSupportedException or PathTooLongException)
        {
            error = new(WindowsPortAudioFailureCode.InvalidPath, "PortAudio 资源路径无效。", false);
            return false;
        }

        return true;
    }

    private static string SanitizeNativeText(nint pointer)
    {
        var value = pointer == 0 ? string.Empty : Marshal.PtrToStringUTF8(pointer) ?? string.Empty;
        if (value.Length > MaxTextCharacters)
        {
            value = value[..MaxTextCharacters];
        }

        var builder = new System.Text.StringBuilder(value.Length);
        foreach (var character in value)
        {
            builder.Append(char.IsControl(character) ? ' ' : character);
        }

        var text = builder.ToString().Trim();
        return string.IsNullOrWhiteSpace(text) ? "未命名设备" : text;
    }

    private static double SanitizeSampleRate(double value) =>
        double.IsFinite(value) && value is >= 8_000 and <= 384_000 ? value : 0;

    private WindowsPortAudioProbeResult Failure(
        WindowsPortAudioFailureCode code,
        string message,
        bool retryable) =>
        new(false, Snapshot, new(code, message, retryable));

    private static WindowsPortAudioProbeResult Failure(
        WindowsPortAudioFailureCode code,
        string message,
        bool retryable,
        WindowsPortAudioSnapshot? snapshot = null) =>
        new(false, snapshot ?? new(
            IsAvailable: false,
            Backend: "portaudio",
            Devices: EmptyDevices,
            DefaultOutputDevice: null,
            ErrorCode: code.ToString(),
            Error: message), new(code, message, retryable));

    private static WindowsPortAudioProbeResult Success(WindowsPortAudioSnapshot snapshot) =>
        new(true, snapshot);

}
