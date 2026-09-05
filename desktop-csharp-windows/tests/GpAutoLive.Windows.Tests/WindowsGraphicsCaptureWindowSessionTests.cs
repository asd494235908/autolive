using System.Runtime.InteropServices;
using System.Threading;
using GpAutoLive.Windows;

namespace GpAutoLive.Windows.Tests;

[TestClass]
[DoNotParallelize]
public sealed class WindowsGraphicsCaptureWindowSessionTests
{
    [TestMethod]
    public async Task StartWithoutBoundHwnd_FailsClosed()
    {
        await using var session = new WindowsGraphicsCaptureWindowSession();
        using var binding = new WindowsVirtualCameraSurfaceBinding();

        var result = await session.StartAsync(binding);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsGraphicsCaptureWindowSessionCode.InvalidBinding, result.Code);
        Assert.IsFalse(result.Snapshot.IsRunning);
    }

    [TestMethod]
    public async Task StartWithStaleOrInvalidHwnd_DoesNotStartCapture()
    {
        await using var session = new WindowsGraphicsCaptureWindowSession();
        using var binding = new WindowsVirtualCameraSurfaceBinding();
        _ = binding.Bind(1);

        var result = await session.StartAsync(binding);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreNotEqual(WindowsGraphicsCaptureWindowSessionCode.Running, result.Code);
        Assert.IsFalse(result.Snapshot.IsRunning);
    }

    [TestMethod]
    public async Task DisposeIsTerminalAndRejectsLaterStart()
    {
        var session = new WindowsGraphicsCaptureWindowSession();
        await session.DisposeAsync();
        using var binding = new WindowsVirtualCameraSurfaceBinding();
        _ = binding.Bind(1);

        var result = await session.StartAsync(binding);

        Assert.IsFalse(result.IsSuccess);
        Assert.AreEqual(WindowsGraphicsCaptureWindowSessionCode.Closed, result.Code);
    }

    [TestMethod]
    public async Task VisibleNativeWindow_StartsRealWgcFramePool()
    {
        if (!OperatingSystem.IsWindows())
        {
            return;
        }

        using var ready = new ManualResetEventSlim(false);
        using var stop = new ManualResetEventSlim(false);
        long handleValue = 0;
        var windowThread = new Thread(() =>
        {
            var created = CreateWindowEx(
                0,
                "STATIC",
                "GpAutoLive WGC integration",
                0x00CF0000,
                0,
                0,
                640,
                360,
                IntPtr.Zero,
                IntPtr.Zero,
                IntPtr.Zero,
                IntPtr.Zero);
            Volatile.Write(ref handleValue, created.ToInt64());
            _ = ShowWindow(created, 5);
            _ = UpdateWindow(created);
            ready.Set();
            while (!stop.Wait(16))
            {
                while (PeekMessageW(out var message, IntPtr.Zero, 0, 0, 1))
                {
                    _ = TranslateMessage(ref message);
                    _ = DispatchMessageW(ref message);
                }

                _ = InvalidateRect(created, IntPtr.Zero, true);
            }

            _ = DestroyWindow(created);
        })
        {
            IsBackground = true,
            Name = "gpautolive-wgc-test-window",
        };
        windowThread.Start();
        Assert.IsTrue(ready.Wait(TimeSpan.FromSeconds(5)));
        var handle = new IntPtr(Volatile.Read(ref handleValue));
        Assert.AreNotEqual(IntPtr.Zero, handle);
        try
        {
            using var binding = new WindowsVirtualCameraSurfaceBinding();
            _ = binding.Bind(unchecked((uint)handle.ToInt64()));
            await using var session = new WindowsGraphicsCaptureWindowSession();

            var result = await session.StartAsync(
                binding,
                TimeSpan.FromSeconds(10));

            Assert.IsTrue(
                result.IsSuccess,
                $"WGC 启动应成功，实际状态为 {result.Code}。");
            Assert.AreEqual(WindowsGraphicsCaptureWindowSessionCode.Running, result.Code);
            await Task.Delay(500);
            Assert.IsTrue(session.Snapshot.IsRunning);
            Assert.IsTrue(session.Snapshot.Width > 0);
            Assert.IsTrue(session.Snapshot.Height > 0);
            await session.StopAsync();
            Assert.AreEqual(WindowsGraphicsCaptureWindowSessionCode.Stopped, session.Snapshot.Code);
        }
        finally
        {
            stop.Set();
            Assert.IsTrue(windowThread.Join(TimeSpan.FromSeconds(2)));
        }
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateWindowEx(
        uint extendedStyle,
        string className,
        string windowName,
        uint style,
        int x,
        int y,
        int width,
        int height,
        IntPtr parent,
        IntPtr menu,
        IntPtr instance,
        IntPtr parameter);

    [DllImport("user32.dll", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool ShowWindow(IntPtr window, int command);

    [DllImport("user32.dll", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool UpdateWindow(IntPtr window);

    [DllImport("user32.dll", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyWindow(IntPtr window);

    [StructLayout(LayoutKind.Sequential)]
    private struct NativeMessage
    {
        public IntPtr HWnd;
        public uint Message;
        public UIntPtr WParam;
        public IntPtr LParam;
        public uint Time;
        public int PointX;
        public int PointY;
    }

    [DllImport("user32.dll", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool PeekMessageW(
        out NativeMessage message,
        IntPtr window,
        uint filterMinimum,
        uint filterMaximum,
        uint removeMessage);

    [DllImport("user32.dll", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool TranslateMessage(ref NativeMessage message);

    [DllImport("user32.dll", ExactSpelling = true)]
    private static extern IntPtr DispatchMessageW(ref NativeMessage message);

    [DllImport("user32.dll", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool InvalidateRect(IntPtr window, IntPtr rectangle, bool erase);
}
