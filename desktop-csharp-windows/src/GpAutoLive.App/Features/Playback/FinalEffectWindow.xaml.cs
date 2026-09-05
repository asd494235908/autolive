using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace GpAutoLive.App.Features.Playback;

public partial class FinalEffectWindow : Window
{
    private readonly FinalEffectWindowController _controller;
    private WindowStyle _fullscreenPreviousStyle;
    private ResizeMode _fullscreenPreviousResizeMode;
    private WindowState _fullscreenPreviousState;

    public IntPtr VideoSurfaceHandle => VideoSurface.SurfaceHandle;

    public bool IsFullscreen { get; private set; }

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
        UpdateLayout();
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
            return;
        }

        WindowState = _fullscreenPreviousState;
        ResizeMode = _fullscreenPreviousResizeMode;
        WindowStyle = _fullscreenPreviousStyle;
        IsFullscreen = false;
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
        AudioSurface.Visibility = snapshot.IsPureAudio ? Visibility.Visible : Visibility.Collapsed;
        EmptySurface.Visibility = snapshot.HasMedia ? Visibility.Collapsed : Visibility.Visible;
        SurfaceModeText.Text = snapshot.SurfaceLabel;
        PlaybackStateText.Text = snapshot.PlaybackLabel;
        PlaybackProgress.Value = snapshot.Progress;
        IdentityText.Text = $"会话 {snapshot.PlaybackGeneration} · 源 {snapshot.SourceRevision} · 循环 {snapshot.LoopIndex}";
        PlayPauseButton.IsEnabled = snapshot.HasMedia;
        StopButton.IsEnabled = snapshot.HasMedia;
    }

    private void PlayPauseButton_Click(object sender, RoutedEventArgs e) =>
        _controller.RequestCommand(FinalEffectPlaybackCommand.TogglePlayPause);

    private void StopButton_Click(object sender, RoutedEventArgs e) =>
        _controller.RequestCommand(FinalEffectPlaybackCommand.Stop);

    private void CloseButton_Click(object sender, RoutedEventArgs e) => Close();

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
        _controller.StateChanged -= Controller_StateChanged;
        _controller.Close();
    }

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

    [StructLayout(LayoutKind.Sequential)]
    private struct NativeRect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }
}
