using System.ComponentModel;
using System.Runtime.InteropServices;
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

    public IntPtr SurfaceHandle => _surfaceHandle;

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
