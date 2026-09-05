using System.Runtime.InteropServices;

namespace GpAutoLive.Windows;

/// <summary>
/// PortAudio v19 的最小动态 ABI。只暴露枚举与输出流需要的函数，避免 P/Invoke 搜索 PATH。
/// </summary>
internal sealed class WindowsPortAudioNative : IDisposable
{
    internal const uint Float32SampleFormat = 0x00000001;
    internal const int Continue = 0;
    internal const int Complete = 1;
    internal const int Abort = 2;

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal delegate int StreamCallback(
        nint inputBuffer,
        nint outputBuffer,
        uint frameCount,
        nint timeInfo,
        uint statusFlags,
        nint userData);

    [StructLayout(LayoutKind.Sequential)]
    internal struct DeviceInfo
    {
        public int StructVersion;
        public nint Name;
        public int HostApi;
        public int MaxInputChannels;
        public int MaxOutputChannels;
        public double DefaultLowInputLatency;
        public double DefaultLowOutputLatency;
        public double DefaultHighInputLatency;
        public double DefaultHighOutputLatency;
        public double DefaultSampleRate;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct HostApiInfo
    {
        public int StructVersion;
        public nint Name;
        public int Type;
        public int DeviceCount;
        public int DefaultInputDevice;
        public int DefaultOutputDevice;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct StreamParameters
    {
        public int Device;
        public int ChannelCount;
        public uint SampleFormat;
        public double SuggestedLatency;
        public nint HostApiSpecificStreamInfo;
    }

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int InitializeDelegate();

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int TerminateDelegate();

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int GetDeviceCountDelegate();

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int GetDefaultOutputDeviceDelegate();

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate nint GetDeviceInfoDelegate(int index);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate nint GetHostApiInfoDelegate(int index);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int OpenStreamDelegate(
        out nint stream,
        nint inputParameters,
        nint outputParameters,
        double sampleRate,
        uint framesPerBuffer,
        uint streamFlags,
        StreamCallback callback,
        nint userData);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int StartStreamDelegate(nint stream);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int StopStreamDelegate(nint stream);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int CloseStreamDelegate(nint stream);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int IsStreamActiveDelegate(nint stream);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate int IsStreamStoppedDelegate(nint stream);

    private readonly nint _library;
    private readonly InitializeDelegate _initialize;
    private readonly TerminateDelegate _terminate;
    private readonly GetDeviceCountDelegate _getDeviceCount;
    private readonly GetDefaultOutputDeviceDelegate _getDefaultOutputDevice;
    private readonly GetDeviceInfoDelegate _getDeviceInfo;
    private readonly GetHostApiInfoDelegate _getHostApiInfo;
    private readonly OpenStreamDelegate _openStream;
    private readonly StartStreamDelegate _startStream;
    private readonly StopStreamDelegate _stopStream;
    private readonly CloseStreamDelegate _closeStream;
    private readonly IsStreamActiveDelegate? _isStreamActive;
    private readonly IsStreamStoppedDelegate? _isStreamStopped;
    private bool _disposed;

    private WindowsPortAudioNative(
        nint library,
        InitializeDelegate initialize,
        TerminateDelegate terminate,
        GetDeviceCountDelegate getDeviceCount,
        GetDefaultOutputDeviceDelegate getDefaultOutputDevice,
        GetDeviceInfoDelegate getDeviceInfo,
        GetHostApiInfoDelegate getHostApiInfo,
        OpenStreamDelegate openStream,
        StartStreamDelegate startStream,
        StopStreamDelegate stopStream,
        CloseStreamDelegate closeStream,
        IsStreamActiveDelegate? isStreamActive,
        IsStreamStoppedDelegate? isStreamStopped)
    {
        _library = library;
        _initialize = initialize;
        _terminate = terminate;
        _getDeviceCount = getDeviceCount;
        _getDefaultOutputDevice = getDefaultOutputDevice;
        _getDeviceInfo = getDeviceInfo;
        _getHostApiInfo = getHostApiInfo;
        _openStream = openStream;
        _startStream = startStream;
        _stopStream = stopStream;
        _closeStream = closeStream;
        _isStreamActive = isStreamActive;
        _isStreamStopped = isStreamStopped;
    }

    internal int Initialize() => _initialize();

    internal int Terminate() => _terminate();

    internal int GetDeviceCount() => _getDeviceCount();

    internal int GetDefaultOutputDevice() => _getDefaultOutputDevice();

    internal nint GetDeviceInfo(int index) => _getDeviceInfo(index);

    internal HostApiInfo GetHostApiInfo(int index)
    {
        var pointer = _getHostApiInfo(index);
        return pointer == 0 ? default : Marshal.PtrToStructure<HostApiInfo>(pointer);
    }

    internal int OpenOutputStream(
        out nint stream,
        ref StreamParameters outputParameters,
        double sampleRate,
        uint framesPerBuffer,
        StreamCallback callback)
    {
        var parameterPointer = Marshal.AllocHGlobal(Marshal.SizeOf<StreamParameters>());
        try
        {
            Marshal.StructureToPtr(outputParameters, parameterPointer, false);
            return _openStream(
                out stream,
                0,
                parameterPointer,
                sampleRate,
                framesPerBuffer,
                0,
                callback,
                0);
        }
        finally
        {
            Marshal.FreeHGlobal(parameterPointer);
        }
    }

    internal int OpenInputStream(
        out nint stream,
        ref StreamParameters inputParameters,
        double sampleRate,
        uint framesPerBuffer,
        StreamCallback callback)
    {
        var parameterPointer = Marshal.AllocHGlobal(Marshal.SizeOf<StreamParameters>());
        try
        {
            Marshal.StructureToPtr(inputParameters, parameterPointer, false);
            return _openStream(
                out stream,
                parameterPointer,
                0,
                sampleRate,
                framesPerBuffer,
                0,
                callback,
                0);
        }
        finally
        {
            Marshal.FreeHGlobal(parameterPointer);
        }
    }

    internal int StartStream(nint stream) => _startStream(stream);

    internal int StopStream(nint stream) => _stopStream(stream);

    internal int CloseStream(nint stream) => _closeStream(stream);

    /// <summary>
    /// 查询原生流健康状态。旧版或裁剪版 PortAudio 缺少可选导出时返回 false，
    /// 调用方应把状态标记为 Unknown，而不是把缺失探针当成设备故障。
    /// </summary>
    internal bool TryQueryStreamState(nint stream, out int active, out int stopped)
    {
        active = 0;
        stopped = 0;
        if (_isStreamActive is null || _isStreamStopped is null || stream == 0)
        {
            return false;
        }

        active = _isStreamActive(stream);
        stopped = _isStreamStopped(stream);
        return true;
    }

    /// <summary>
    /// 读取 PortAudio callback 的 PaStreamCallbackTimeInfo。该结构是三个连续的 double；
    /// 只读取 currentTime 和 outputBufferDacTime，避免在实时回调中创建托管对象。
    /// </summary>
    internal static bool TryReadCallbackTimeInfo(
        nint timeInfo,
        out double currentTime,
        out double outputBufferDacTime)
    {
        currentTime = 0;
        outputBufferDacTime = 0;
        if (timeInfo == 0)
        {
            return false;
        }

        try
        {
            currentTime = BitConverter.Int64BitsToDouble(Marshal.ReadInt64(timeInfo, sizeof(double)));
            outputBufferDacTime = BitConverter.Int64BitsToDouble(Marshal.ReadInt64(timeInfo, sizeof(double) * 2));
            return double.IsFinite(currentTime) && double.IsFinite(outputBufferDacTime);
        }
        catch (Exception exception) when (exception is AccessViolationException
            or ArgumentException
            or SEHException)
        {
            currentTime = 0;
            outputBufferDacTime = 0;
            return false;
        }
    }

    internal bool TryInitialize(out int errorCode)
    {
        errorCode = Initialize();
        return errorCode == 0;
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }

        _disposed = true;
        NativeLibrary.Free(_library);
    }

    internal static bool TryLoad(string path, out WindowsPortAudioNative? native)
    {
        native = null;
        nint library = 0;
        try
        {
            library = NativeLibrary.Load(path);
            native = new(
                library,
                GetDelegate<InitializeDelegate>(library, "Pa_Initialize"),
                GetDelegate<TerminateDelegate>(library, "Pa_Terminate"),
                GetDelegate<GetDeviceCountDelegate>(library, "Pa_GetDeviceCount"),
                GetDelegate<GetDefaultOutputDeviceDelegate>(library, "Pa_GetDefaultOutputDevice"),
                GetDelegate<GetDeviceInfoDelegate>(library, "Pa_GetDeviceInfo"),
                GetDelegate<GetHostApiInfoDelegate>(library, "Pa_GetHostApiInfo"),
                GetDelegate<OpenStreamDelegate>(library, "Pa_OpenStream"),
                GetDelegate<StartStreamDelegate>(library, "Pa_StartStream"),
                GetDelegate<StopStreamDelegate>(library, "Pa_StopStream"),
                GetDelegate<CloseStreamDelegate>(library, "Pa_CloseStream"),
                GetOptionalDelegate<IsStreamActiveDelegate>(library, "Pa_IsStreamActive"),
                GetOptionalDelegate<IsStreamStoppedDelegate>(library, "Pa_IsStreamStopped"));
            return true;
        }
        catch (Exception exception) when (exception is ArgumentException
            or BadImageFormatException
            or DllNotFoundException
            or EntryPointNotFoundException
            or FileLoadException
            or InvalidOperationException
            or UnauthorizedAccessException
            or System.Security.SecurityException)
        {
            if (library != 0)
            {
                NativeLibrary.Free(library);
            }

            return false;
        }
    }

    private static T GetDelegate<T>(nint library, string exportName)
        where T : Delegate
    {
        var pointer = NativeLibrary.GetExport(library, exportName);
        return Marshal.GetDelegateForFunctionPointer<T>(pointer);
    }

    private static T? GetOptionalDelegate<T>(nint library, string exportName)
        where T : Delegate
    {
        return NativeLibrary.TryGetExport(library, exportName, out var pointer)
            ? Marshal.GetDelegateForFunctionPointer<T>(pointer)
            : null;
    }
}
