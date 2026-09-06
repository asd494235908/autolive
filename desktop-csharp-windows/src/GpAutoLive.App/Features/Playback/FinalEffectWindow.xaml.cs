using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace GpAutoLive.App.Features.Playback;

public partial class FinalEffectWindow : Window
{
    private const int WmSizing = 0x0214;
    private const int WmszLeft = 1;
    private const int WmszRight = 2;
    private const int WmszTop = 3;
    private const int WmszTopLeft = 4;
    private const int WmszTopRight = 5;
    private const int WmszBottom = 6;
    private const int WmszBottomLeft = 7;
    private const int WmszBottomRight = 8;
    private const uint MonitorDefaultToNearest = 2;
    private const uint DefaultDpi = 96;

    private readonly FinalEffectWindowController _controller;
    private WindowStyle _fullscreenPreviousStyle;
    private ResizeMode _fullscreenPreviousResizeMode;
    private WindowState _fullscreenPreviousState;
    private HwndSource? _windowSource;
    private (uint Width, uint Height)? _videoDimensions;
    private (uint Width, uint Height)? _lastSizedVideoDimensions;
    private double _videoAspectRatio = FinalEffectWindowSizing.DefaultAspectRatio;
    private bool _isApplyingVideoWindowSize;

    public IntPtr VideoSurfaceHandle => VideoSurface.SurfaceHandle;

    public bool IsFullscreen { get; private set; }

    internal double VideoAspectRatio => _videoAspectRatio;

    internal bool IsVideoAspectRatioLocked => _videoDimensions is not null;

    public FinalEffectWindow(FinalEffectWindowController controller)
    {
        _controller = controller ?? throw new ArgumentNullException(nameof(controller));
        InitializeComponent();
        _controller.StateChanged += Controller_StateChanged;
        ApplySnapshot(_controller.Snapshot);
    }

    public bool TryGetVideoSurfaceHandle(out uint handle)
    {
        VideoSurface.UpdateLayout();
        return TryGetReadyHandle(VideoSurfaceHandle, out handle);
    }

    /// <summary>
    /// 返回最终效果窗口的顶层 HWND，供 Windows.Graphics.Capture 绑定。
    /// mpv 仍绑定 <see cref="VideoSurfaceHandle"/>；WGC 不绑定其子 HWND。
    /// </summary>
    public bool TryGetCaptureWindowHandle(out uint handle)
    {
        RefreshPresentationLayout();
        return TryGetReadyHandle(new WindowInteropHelper(this).Handle, out handle);
    }

    /// <summary>切换最终效果窗口的无边框全屏；不创建第二个窗口或渲染表面。</summary>
    public void ToggleFullscreen()
    {
        if (!IsFullscreen)
        {
            _fullscreenPreviousStyle = WindowStyle;
            _fullscreenPreviousResizeMode = ResizeMode;
            _fullscreenPreviousState = WindowState;
            WindowStyle = WindowStyle.None;
            ResizeMode = ResizeMode.NoResize;
            WindowState = WindowState.Maximized;
            IsFullscreen = true;
            RefreshPresentationLayout();
            return;
        }

        WindowState = _fullscreenPreviousState;
        ResizeMode = _fullscreenPreviousResizeMode;
        WindowStyle = _fullscreenPreviousStyle;
        IsFullscreen = false;
        ApplyVideoWindowSize(force: true);
        RefreshPresentationLayout();
    }

    private void Window_Loaded(object sender, RoutedEventArgs e)
    {
        ApplySnapshot(_controller.Snapshot);
        ApplyVideoWindowSize(force: true);
        RefreshPresentationLayout();
    }

    private void Window_SourceInitialized(object? sender, EventArgs e)
    {
        _windowSource = PresentationSource.FromVisual(this) as HwndSource;
        _windowSource?.AddHook(WindowMessageHook);
        ApplySnapshot(_controller.Snapshot);
        ApplyVideoWindowSize(force: true);
    }

    private void Controller_StateChanged(object? sender, EventArgs e)
    {
        if (Dispatcher.CheckAccess())
        {
            ApplySnapshot(_controller.Snapshot);
            return;
        }

        _ = Dispatcher.InvokeAsync(() => ApplySnapshot(_controller.Snapshot));
    }

    private void ApplySnapshot(FinalEffectSnapshot snapshot)
    {
        VideoSurface.Visibility = snapshot.SurfaceKind is FinalEffectSurfaceKind.VideoHwndReserved
            ? Visibility.Visible
            : Visibility.Collapsed;
        AudioSurface.Visibility = snapshot.SurfaceKind is FinalEffectSurfaceKind.AudioBlack
            ? Visibility.Visible
            : Visibility.Collapsed;

        _videoDimensions = snapshot.SurfaceKind is FinalEffectSurfaceKind.VideoHwndReserved
            && snapshot.VideoWidth is uint width and > 0
            && snapshot.VideoHeight is uint height and > 0
                ? (width, height)
                : null;
        _videoAspectRatio = FinalEffectWindowSizing.ResolveAspectRatio(
            snapshot.VideoWidth,
            snapshot.VideoHeight);
        if (_videoDimensions is null)
        {
            _lastSizedVideoDimensions = null;
            return;
        }

        ApplyVideoWindowSize(force: false);
    }

    private void RefreshPresentationLayout()
    {
        UpdateLayout();
        VideoSurface.UpdateLayout();
    }

    private void Window_PreviewKeyDown(object sender, System.Windows.Input.KeyEventArgs e)
    {
        if (e.Key == System.Windows.Input.Key.F11
            || (e.Key == System.Windows.Input.Key.Escape && IsFullscreen))
        {
            ToggleFullscreen();
            e.Handled = true;
        }
    }

    private void Window_Closed(object? sender, EventArgs e)
    {
        _windowSource?.RemoveHook(WindowMessageHook);
        _windowSource = null;
        _controller.StateChanged -= Controller_StateChanged;
        _controller.Close();
    }

    private void ApplyVideoWindowSize(bool force)
    {
        if (_windowSource is null
            || _videoDimensions is not (uint width, uint height)
            || IsFullscreen
            || WindowState is WindowState.Maximized or WindowState.Minimized
            || _isApplyingVideoWindowSize
            || (!force && _lastSizedVideoDimensions == (width, height)))
        {
            return;
        }

        var windowHandle = new WindowInteropHelper(this).Handle;
        if (windowHandle == IntPtr.Zero)
        {
            return;
        }

        var workArea = GetWorkAreaInDips(windowHandle);
        var frame = GetFrameSizeInDips(windowHandle);
        var target = FinalEffectWindowSizing.CalculateInitialWindowSize(
            width,
            height,
            workArea.Width,
            workArea.Height,
            frame.Width,
            frame.Height);

        _isApplyingVideoWindowSize = true;
        try
        {
            Width = target.Width;
            Height = target.Height;
            _lastSizedVideoDimensions = (width, height);
        }
        finally
        {
            _isApplyingVideoWindowSize = false;
        }
    }

    private IntPtr WindowMessageHook(
        IntPtr hwnd,
        int message,
        IntPtr wParam,
        IntPtr lParam,
        ref bool handled)
    {
        if (message != WmSizing
            || _videoDimensions is null
            || IsFullscreen
            || _isApplyingVideoWindowSize
            || lParam == IntPtr.Zero)
        {
            return IntPtr.Zero;
        }

        if (!TryGetFrameSizeInPixels(hwnd, out var frameWidth, out var frameHeight))
        {
            return IntPtr.Zero;
        }

        var rect = Marshal.PtrToStructure<NativeRect>(lParam);
        var clientWidth = Math.Max(1, rect.Right - rect.Left - frameWidth);
        var clientHeight = Math.Max(1, rect.Bottom - rect.Top - frameHeight);
        var dpi = GetDpiForWindow(hwnd);
        var minimumClientWidth = DipsToPixels(FinalEffectWindowSizing.MinimumClientWidth, dpi);
        var minimumClientHeight = DipsToPixels(FinalEffectWindowSizing.MinimumClientHeight, dpi);
        var edge = unchecked((int)wParam.ToInt64());
        var requested = edge is WmszTop or WmszBottom
            ? FinalEffectWindowSizing.CalculateClientSizeFromHeight(
                clientHeight,
                _videoAspectRatio,
                minimumClientWidth,
                minimumClientHeight)
            : FinalEffectWindowSizing.CalculateClientSizeFromWidth(
                clientWidth,
                _videoAspectRatio,
                minimumClientWidth,
                minimumClientHeight);
        var outerWidth = checked(requested.Width + frameWidth);
        var outerHeight = checked(requested.Height + frameHeight);

        switch (edge)
        {
            case WmszLeft:
                rect.Left = rect.Right - outerWidth;
                rect.Bottom = rect.Top + outerHeight;
                break;
            case WmszRight:
                rect.Right = rect.Left + outerWidth;
                rect.Bottom = rect.Top + outerHeight;
                break;
            case WmszTop:
                rect.Top = rect.Bottom - outerHeight;
                rect.Right = rect.Left + outerWidth;
                break;
            case WmszBottom:
                rect.Right = rect.Left + outerWidth;
                rect.Bottom = rect.Top + outerHeight;
                break;
            case WmszTopLeft:
                rect.Left = rect.Right - outerWidth;
                rect.Top = rect.Bottom - outerHeight;
                break;
            case WmszTopRight:
                rect.Right = rect.Left + outerWidth;
                rect.Top = rect.Bottom - outerHeight;
                break;
            case WmszBottomLeft:
                rect.Left = rect.Right - outerWidth;
                rect.Bottom = rect.Top + outerHeight;
                break;
            case WmszBottomRight:
                rect.Right = rect.Left + outerWidth;
                rect.Bottom = rect.Top + outerHeight;
                break;
            default:
                return IntPtr.Zero;
        }

        Marshal.StructureToPtr(rect, lParam, fDeleteOld: false);
        handled = true;
        return IntPtr.Zero;
    }

    private static (double Width, double Height) GetWorkAreaInDips(IntPtr windowHandle)
    {
        var monitor = MonitorFromWindow(windowHandle, MonitorDefaultToNearest);
        if (monitor != IntPtr.Zero)
        {
            var monitorInfo = new MonitorInfo { Size = (uint)Marshal.SizeOf<MonitorInfo>() };
            if (GetMonitorInfo(monitor, ref monitorInfo))
            {
                var dpi = GetDpiForWindow(windowHandle);
                return (
                    PixelsToDips(monitorInfo.Work.Right - monitorInfo.Work.Left, dpi),
                    PixelsToDips(monitorInfo.Work.Bottom - monitorInfo.Work.Top, dpi));
            }
        }

        return (SystemParameters.WorkArea.Width, SystemParameters.WorkArea.Height);
    }

    private static (double Width, double Height) GetFrameSizeInDips(IntPtr windowHandle)
    {
        if (!TryGetFrameSizeInPixels(windowHandle, out var width, out var height))
        {
            return (0, 0);
        }

        var dpi = GetDpiForWindow(windowHandle);
        return (PixelsToDips(width, dpi), PixelsToDips(height, dpi));
    }

    private static bool TryGetFrameSizeInPixels(
        IntPtr windowHandle,
        out int width,
        out int height)
    {
        width = 0;
        height = 0;
        if (!GetWindowRect(windowHandle, out var windowRect)
            || !GetClientRect(windowHandle, out var clientRect))
        {
            return false;
        }

        width = Math.Max(0, windowRect.Right - windowRect.Left - clientRect.Right + clientRect.Left);
        height = Math.Max(0, windowRect.Bottom - windowRect.Top - clientRect.Bottom + clientRect.Top);
        return true;
    }

    private static int DipsToPixels(int value, uint dpi) =>
        checked((int)Math.Round(value * Math.Max(dpi, DefaultDpi) / (double)DefaultDpi));

    private static double PixelsToDips(int value, uint dpi) =>
        value * (double)DefaultDpi / Math.Max(dpi, DefaultDpi);

    private static bool TryGetReadyHandle(IntPtr value, out uint handle)
    {
        var numericValue = value.ToInt64();
        if (value == IntPtr.Zero
            || numericValue <= 0
            || numericValue > uint.MaxValue
            || !IsWindow(value)
            || !IsWindowVisible(value)
            || !GetClientRect(value, out var clientRect)
            || clientRect.Right <= clientRect.Left
            || clientRect.Bottom <= clientRect.Top)
        {
            handle = 0;
            return false;
        }

        handle = unchecked((uint)numericValue);
        return true;
    }

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool IsWindow(IntPtr handle);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool IsWindowVisible(IntPtr handle);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetClientRect(IntPtr handle, out NativeRect rect);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetWindowRect(IntPtr handle, out NativeRect rect);

    [DllImport("user32.dll")]
    private static extern IntPtr MonitorFromWindow(IntPtr handle, uint flags);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo monitorInfo);

    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr handle);

    [StructLayout(LayoutKind.Sequential)]
    private struct MonitorInfo
    {
        public uint Size;
        public NativeRect Monitor;
        public NativeRect Work;
        public uint Flags;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct NativeRect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }
}
