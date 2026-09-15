using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

internal sealed class FakeWindowsPortAudioStreamNative : IWindowsPortAudioStreamNative
{
    internal string? BlockOperation { get; set; }
    internal ManualResetEventSlim Entered { get; } = new();
    internal ManualResetEventSlim Release { get; } = new();
    internal int CloseError { get; set; }
    internal int CloseCount;
    internal int DisposeCount;
    internal int MaxConcurrent;
    private int _concurrent;
    private bool _active;

    private int Call(string name, int result = 0)
    {
        var concurrent = Interlocked.Increment(ref _concurrent);
        MaxConcurrent = Math.Max(MaxConcurrent, concurrent);
        try
        {
            if (BlockOperation == name)
            {
                Entered.Set();
                if (!Release.Wait(TimeSpan.FromSeconds(10)))
                {
                    throw new TimeoutException("测试原生调用未释放。");
                }
            }

            return result;
        }
        finally
        {
            Interlocked.Decrement(ref _concurrent);
        }
    }

    public bool TryInitialize(out int errorCode)
    {
        errorCode = Call("Initialize");
        return errorCode == 0;
    }

    public int OpenInputStream(out nint stream, ref WindowsPortAudioNative.StreamParameters parameters,
        double sampleRate, uint framesPerBuffer, WindowsPortAudioNative.StreamCallback callback)
    {
        var result = Call("Open");
        stream = 123;
        return result;
    }

    public int OpenOutputStream(out nint stream, ref WindowsPortAudioNative.StreamParameters parameters,
        double sampleRate, uint framesPerBuffer, WindowsPortAudioNative.StreamCallback callback) =>
        OpenInputStream(out stream, ref parameters, sampleRate, framesPerBuffer, callback);

    public int StartStream(nint stream)
    {
        var result = Call("Start");
        _active = true;
        return result;
    }

    public int StopStream(nint stream)
    {
        var result = Call("Stop");
        _active = false;
        return result;
    }

    public int CloseStream(nint stream)
    {
        Interlocked.Increment(ref CloseCount);
        return Call("Close", CloseError);
    }

    public int Terminate() => Call("Terminate");

    public bool TryQueryStreamState(nint stream, out int active, out int stopped)
    {
        Call("Query");
        active = _active ? 1 : 0;
        stopped = _active ? 0 : 1;
        return true;
    }

    public void Dispose() => Interlocked.Increment(ref DisposeCount);
}
