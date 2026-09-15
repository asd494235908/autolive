using System.Buffers.Binary;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace GpAutoLive.App.Features.Playback;

/// <summary>只绘制 mpv 最终画面；与 mpv 宿主同级的原生覆盖窗口不受播放器子窗口销毁影响。</summary>
internal sealed class HeldVideoFrameWindow : IDisposable
{
    private readonly HwndSource _window;
    private readonly byte[] _pixels;
    private BitmapInfo _bitmap;
    private bool _paintSucceeded;

    public bool IsAlive => !_window.IsDisposed && IsWindow(_window.Handle);

    public HeldVideoFrameWindow(IntPtr parent, byte[] pngBytes)
    {
        // 与 mpv IPC 截图入口一致：单边最大 8192，总像素最大 16,777,216 / 64 MiB BGRA。
        if (pngBytes.Length is < 33 or > 32 * 1024 * 1024 ||
            !pngBytes.AsSpan(0, 8).SequenceEqual(new byte[] { 137, 80, 78, 71, 13, 10, 26, 10 }) ||
            BinaryPrimitives.ReadUInt32BigEndian(pngBytes.AsSpan(8, 4)) != 13 ||
            !pngBytes.AsSpan(12, 4).SequenceEqual("IHDR"u8))
            throw new InvalidDataException("保帧图像不是有效的有界 PNG。");
        var width = BinaryPrimitives.ReadUInt32BigEndian(pngBytes.AsSpan(16, 4));
        var height = BinaryPrimitives.ReadUInt32BigEndian(pngBytes.AsSpan(20, 4));
        if (width is 0 or > 8192 || height is 0 or > 8192 || (ulong)width * height > 16_777_216)
            throw new InvalidDataException("保帧图像单边必须在 1～8192 像素内，总像素不能超过 16,777,216。");
        using var input = new MemoryStream(pngBytes, writable: false);
        var decoder = new PngBitmapDecoder(input, BitmapCreateOptions.None, BitmapCacheOption.OnLoad);
        var frame = decoder.Frames[0];
        if (frame.PixelWidth != width || frame.PixelHeight != height)
            throw new InvalidDataException("保帧图像尺寸不一致。");
        var bitmap = new FormatConvertedBitmap(frame, PixelFormats.Bgra32, null, 0);
        var stride = checked((int)width * 4);
        _pixels = new byte[checked(stride * (int)height)];
        bitmap.CopyPixels(_pixels, stride, 0);
        _bitmap = new BitmapInfo
        {
            Size = (uint)Marshal.SizeOf<BitmapInfo>(), Width = (int)width, Height = -(int)height,
            Planes = 1, BitCount = 32
        };
        var overlayParent = GetParent(parent);
        if (overlayParent == IntPtr.Zero)
            throw new InvalidOperationException("视频表面的所属窗口尚未创建。");
        _window = new HwndSource(new HwndSourceParameters("GpAutoLive held video frame")
        {
            ParentWindow = overlayParent, WindowStyle = 0x40000000 | 0x04000000,
            ExtendedWindowStyle = 0x08000000, Width = 1, Height = 1
        });
        _window.AddHook(WindowProc);
    }

    public void Show(IntPtr parent, bool requirePaint = true)
    {
        if (!GetClientRect(parent, out var rect) || rect.Right <= 0 || rect.Bottom <= 0)
            throw new InvalidOperationException("视频表面尚无可显示区域。");
        var target = rect;
        // 将视频区域映射到共同父 HWND，覆盖层不能越出视频表面遮住其他 WPF 内容。
        Marshal.SetLastPInvokeError(0);
        if (MapWindowPoints(parent, GetParent(_window.Handle), ref target, 2) == 0 && Marshal.GetLastPInvokeError() != 0)
            throw new Win32Exception(Marshal.GetLastPInvokeError(), "无法定位视频保帧区域。");
        if (!SetWindowPos(_window.Handle, IntPtr.Zero, target.Left, target.Top, rect.Right, rect.Bottom, 0x0010))
            throw new Win32Exception(Marshal.GetLastWin32Error(), "无法显示保帧表面。");
        // 首次显示前也先画好像素，避免新 HWND 可见而首个 WM_PAINT 尚未到达。
        var dc = GetDC(_window.Handle);
        try { _paintSucceeded = Draw(dc, rect); }
        finally { if (dc != IntPtr.Zero) _ = ReleaseDC(_window.Handle, dc); }
        if (!_paintSucceeded || !SetWindowPos(_window.Handle, IntPtr.Zero, target.Left, target.Top,
                rect.Right, rect.Bottom, 0x0010 | 0x0040))
            throw new InvalidOperationException("保帧像素未能显示，保持当前视频源。");
        // 同步完成 WM_PAINT，调用方才能卸载旧源。WM_ERASEBKGND 禁止先清黑。
        _paintSucceeded = false;
        if (!RedrawWindow(_window.Handle, IntPtr.Zero, IntPtr.Zero, 0x0001 | 0x0100))
            throw new Win32Exception(Marshal.GetLastWin32Error(), "无法重绘保帧表面。");
        // 已存在的覆盖层可能在宿主退出/重排时暂时被裁剪，此时 RDW_UPDATENOW
        // 不保证派发 WM_PAINT；上面的 GetDC 绘制已成功。首次显示仍必须确认 WM_PAINT。
        if (requirePaint && !_paintSucceeded)
            throw new InvalidOperationException("保帧像素未能绘制，保持当前视频源。");
    }

    private IntPtr WindowProc(IntPtr hwnd, int message, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (message == 0x0014) { handled = true; return new IntPtr(1); }
        if (message == 0x0084) { handled = true; return new IntPtr(-1); } // 不抢焦点或鼠标。
        if (message != 0x000f) return IntPtr.Zero;
        var dc = BeginPaint(hwnd, out var paint);
        try
        {
            if (dc != IntPtr.Zero && GetClientRect(hwnd, out var rect))
            {
                _paintSucceeded = Draw(dc, rect);
            }
        }
        finally { _ = EndPaint(hwnd, ref paint); }
        handled = true;
        return IntPtr.Zero;
    }

    public void Dispose() => _window.Dispose();

    private bool Draw(IntPtr dc, Rect rect)
    {
        if (dc == IntPtr.Zero) return false;
        var painted = StretchDIBits(dc, 0, 0, rect.Right, rect.Bottom, 0, 0, _bitmap.Width,
            -_bitmap.Height, _pixels, ref _bitmap, 0, 0x00cc0020);
        return painted is not 0 and not -1;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BitmapInfo
    {
        public uint Size;
        public int Width, Height;
        public ushort Planes, BitCount;
        public uint Compression, ImageSize;
        public int XPelsPerMeter, YPelsPerMeter;
        public uint ColorsUsed, ColorsImportant;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    private struct PaintStruct
    {
        public IntPtr Dc;
        public int Erase;
        public Rect Paint;
        public int Restore, IncUpdate;
        [MarshalAs(UnmanagedType.ByValArray, SizeConst = 32)] public byte[] Reserved;
    }

    [DllImport("user32.dll")] private static extern IntPtr BeginPaint(IntPtr hwnd, out PaintStruct paint);
    [DllImport("user32.dll")] private static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern IntPtr GetParent(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError = true)] private static extern int MapWindowPoints(IntPtr from, IntPtr to, ref Rect rect, uint points);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool IsWindow(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern int ReleaseDC(IntPtr hwnd, IntPtr dc);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool EndPaint(IntPtr hwnd, ref PaintStruct paint);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll", SetLastError = true)] [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool RedrawWindow(IntPtr hwnd, IntPtr update, IntPtr region, uint flags);
    [DllImport("gdi32.dll")]
    private static extern int StretchDIBits(IntPtr dc, int x, int y, int width, int height,
        int sourceX, int sourceY, int sourceWidth, int sourceHeight, byte[] bits,
        ref BitmapInfo info, uint usage, uint operation);
}
