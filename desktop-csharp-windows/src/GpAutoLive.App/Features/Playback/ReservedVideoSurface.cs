using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace GpAutoLive.App.Features.Playback;

/// <summary>
/// 为 Windows mpv 后端提供的受控子 HWND。
/// 没有受管媒体进程时只显示黑色 STATIC 表面，不自行加载 mpv、FFmpeg 或媒体内容。
/// </summary>
public sealed class ReservedVideoSurface : HwndHost
{
    private const int WsChild = 0x40000000;
    private const int WsVisible = 0x10000000;
    private const int WsClipSiblings = 0x04000000;
    private const int WsClipChildren = 0x02000000;
    private const int SsBlackRect = 0x00000004;

    private IntPtr _surfaceHandle;
    private HeldVideoFrameWindow? _heldFrame;
    private Window? _heldFrameOwner;

    public IntPtr SurfaceHandle => _surfaceHandle;

    public bool HasHeldFrame { get { Dispatcher.VerifyAccess(); return _heldFrame?.IsAlive == true; } }

    public bool TryHoldFrame(byte[] pngBytes, out string? error)
    {
        Dispatcher.VerifyAccess();
        HeldVideoFrameWindow? next = null;
        try
        {
            if (_surfaceHandle == IntPtr.Zero)
                throw new InvalidOperationException("视频表面尚未创建。");
            next = new HeldVideoFrameWindow(_surfaceHandle, pngBytes);
            next.Show(_surfaceHandle);
            _heldFrame?.Dispose();
            _heldFrame = next;
            var owner = Window.GetWindow(this);
            if (!ReferenceEquals(_heldFrameOwner, owner))
            {
                if (_heldFrameOwner is not null) _heldFrameOwner.Closed -= HeldFrameOwner_Closed;
                _heldFrameOwner = owner;
                if (_heldFrameOwner is not null) _heldFrameOwner.Closed += HeldFrameOwner_Closed;
            }
            error = null;
            return true;
        }
        catch (Exception exception) when (exception is ArgumentException or InvalidOperationException or
            IOException or InvalidDataException or FileFormatException or NotSupportedException or System.Runtime.InteropServices.COMException or Win32Exception)
        {
            next?.Dispose();
            error = exception.Message;
            return false;
        }
    }

    public void ReleaseHeldFrame()
    {
        Dispatcher.VerifyAccess();
        if (_heldFrameOwner is not null) _heldFrameOwner.Closed -= HeldFrameOwner_Closed;
        _heldFrameOwner = null;
        _heldFrame?.Dispose();
        _heldFrame = null;
    }

    // HwndHost 的托管销毁可能晚于 Window.Close；在所有者关闭时立即释放像素和子 HWND。
    private void HeldFrameOwner_Closed(object? sender, EventArgs e) => ReleaseHeldFrame();

    protected override IntPtr WndProc(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg is 0x0005 or 0x0047 && _heldFrame is not null && lParam != IntPtr.Zero)
        {
            try { _heldFrame.Show(_surfaceHandle, requirePaint: false); }
            catch (Exception exception) when (exception is InvalidOperationException or Win32Exception)
            {
                // 窗口消息边界不得抛异常；保留已有像素，下次有效 resize 再重绘。
                System.Diagnostics.Trace.TraceWarning("视频保帧表面重绘失败：{0}", exception.Message);
            }
        }
        return base.WndProc(hwnd, msg, wParam, lParam, ref handled);
    }

    protected override HandleRef BuildWindowCore(HandleRef hwndParent)
    {
        _surfaceHandle = CreateWindowEx(
            0,
            "STATIC",
            string.Empty,
            WsChild | WsVisible | WsClipSiblings | WsClipChildren | SsBlackRect,
            0,
            0,
            0,
            0,
            hwndParent.Handle,
            IntPtr.Zero,
            IntPtr.Zero,
            IntPtr.Zero);

        if (_surfaceHandle == IntPtr.Zero)
        {
            throw new Win32Exception(Marshal.GetLastWin32Error(), "无法创建视频表面预留 HWND。");
        }

        return new HandleRef(this, _surfaceHandle);
    }

    protected override void DestroyWindowCore(HandleRef hwnd)
    {
        ReleaseHeldFrame();
        if (hwnd.Handle != IntPtr.Zero)
        {
            _ = DestroyWindow(hwnd.Handle);
        }

        _surfaceHandle = IntPtr.Zero;
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateWindowEx(
        int exStyle,
        string className,
        string windowName,
        int style,
        int x,
        int y,
        int width,
        int height,
        IntPtr parentHandle,
        IntPtr menuHandle,
        IntPtr instanceHandle,
        IntPtr parameter);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyWindow(IntPtr windowHandle);
}
