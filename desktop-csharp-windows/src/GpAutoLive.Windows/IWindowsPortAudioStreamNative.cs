namespace GpAutoLive.Windows;

/// <summary>流拥有者的原生故障注入边界；设备枚举继续使用既有具体 ABI。</summary>
internal interface IWindowsPortAudioStreamNative : IDisposable
{
    bool TryInitialize(out int errorCode);
    int OpenInputStream(out nint stream, ref WindowsPortAudioNative.StreamParameters parameters,
        double sampleRate, uint framesPerBuffer, WindowsPortAudioNative.StreamCallback callback);
    int OpenOutputStream(out nint stream, ref WindowsPortAudioNative.StreamParameters parameters,
        double sampleRate, uint framesPerBuffer, WindowsPortAudioNative.StreamCallback callback);
    int StartStream(nint stream);
    int StopStream(nint stream);
    int CloseStream(nint stream);
    int Terminate();
    bool TryQueryStreamState(nint stream, out int active, out int stopped);
}
