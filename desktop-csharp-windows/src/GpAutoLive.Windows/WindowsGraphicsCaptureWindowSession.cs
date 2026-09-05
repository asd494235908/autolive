using System.Runtime.InteropServices;
using Windows.Graphics.Capture;
using Windows.Graphics.DirectX;
using Windows.Graphics.DirectX.Direct3D11;
using Windows.Foundation;
using Vortice.Direct3D11;
using Vortice.DXGI;

using VorticeDevice = Vortice.Direct3D11.ID3D11Device;
using VorticeDeviceContext = Vortice.Direct3D11.ID3D11DeviceContext;

namespace GpAutoLive.Windows;

/// <summary>WGC HWND 会话的稳定状态分类。</summary>
public enum WindowsGraphicsCaptureWindowSessionCode
{
    Stopped,
    Starting,
    Running,
    NotWindows,
    UnsupportedVersion,
    InvalidBinding,
    InvalidHandle,
    ItemUnavailable,
    RuntimeUnavailable,
    DeviceUnavailable,
    StartFailed,
    BindingInvalidated,
    FrameSizeChanged,
    Cancelled,
    Closed,
}

/// <summary>不包含异常正文、路径、令牌或原生句柄的 WGC 会话快照。</summary>
public sealed record WindowsGraphicsCaptureWindowSessionSnapshot(
    WindowsGraphicsCaptureWindowSessionCode Code,
    uint? WindowId,
    ulong Generation,
    int Width,
    int Height,
    ulong FrameCount,
    long LastTimestamp100Ns)
{
    public bool IsRunning => Code is WindowsGraphicsCaptureWindowSessionCode.Starting
        or WindowsGraphicsCaptureWindowSessionCode.Running;
}

/// <summary>WGC 会话启动结果。</summary>
public sealed record WindowsGraphicsCaptureWindowSessionResult(
    bool IsSuccess,
    WindowsGraphicsCaptureWindowSessionCode Code,
    WindowsGraphicsCaptureWindowSessionSnapshot Snapshot);

/// <summary>
/// 与 WGC frame pool 使用同一硬件 D3D11 设备的同步处理上下文。
/// 回调只在捕获线程内使用；调用方不得跨回调保存对象。
/// </summary>
public sealed class WindowsGraphicsCaptureD3D11Context : IDisposable
{
    internal WindowsGraphicsCaptureD3D11Context(VorticeDevice device, VorticeDeviceContext immediateContext)
    {
        Device = device;
        ImmediateContext = immediateContext;
    }

    public VorticeDevice Device { get; }

    public VorticeDeviceContext ImmediateContext { get; }

    public void Dispose()
    {
        try
        {
            ImmediateContext.Dispose();
        }
        finally
        {
            Device.Dispose();
        }
    }
}

/// <summary>
/// 在受管后台线程创建最终效果 HWND 的真实 WGC frame pool，并消费最新 GPU frame。
/// 本类不做 CPU 像素转换或 sidecar 输出；可通过同步回调把同设备 D3D11 上下文交给 GPU 处理器。
/// </summary>
public sealed class WindowsGraphicsCaptureWindowSession : IAsyncDisposable
{
    private const uint BgraSupport = 0x20;
    private const uint VideoSupport = 0x800;
    private const uint D3D11SdkVersion = 7;
    private const uint UnknownDriverType = 0;
    private const int FramePoolSize = 3;
    private const uint MaxAdapterCount = 32;
    private const int StartupTimeoutMilliseconds = 5_000;
    private const int StopJoinMilliseconds = 2_000;
    private const int EBounds = unchecked((int)0x8000000B);
    private static readonly Guid DxgiDeviceIid = new("54ec77fa-1377-44e6-8c32-88fd5f44c84c");

    private readonly object _gate = new();
    private WindowsGraphicsCaptureWindowSessionSnapshot _snapshot =
        new(WindowsGraphicsCaptureWindowSessionCode.Stopped, null, 0, 0, 0, 0, 0);
    private Thread? _worker;
    private ManualResetEventSlim? _stopSignal;
    private bool _closed;
    private bool _startupCompleted;
    private long _frameCount;
    private long _lastTimestamp100Ns;

    public WindowsGraphicsCaptureWindowSessionSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return _snapshot;
            }
        }
    }

    /// <summary>以当前 HWND 绑定快照创建 WGC 会话；同一实例同时只允许一个会话。</summary>
    public async Task<WindowsGraphicsCaptureWindowSessionResult> StartAsync(
        WindowsVirtualCameraSurfaceBinding binding,
        TimeSpan? timeout = null,
        CancellationToken cancellationToken = default,
        Action<Direct3D11CaptureFrame, WindowsGraphicsCaptureD3D11Context>? frameConsumer = null)
    {
        ArgumentNullException.ThrowIfNull(binding);
        var bindingSnapshot = binding.Snapshot;
        if (!bindingSnapshot.IsBound || bindingSnapshot.WindowId is null)
        {
            return Fail(WindowsGraphicsCaptureWindowSessionCode.InvalidBinding);
        }

        TaskCompletionSource<WindowsGraphicsCaptureWindowSessionResult> startup =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        Thread worker;
        ManualResetEventSlim stopSignal;
        lock (_gate)
        {
            if (_closed)
            {
                return Fail(WindowsGraphicsCaptureWindowSessionCode.Closed);
            }

            if (_worker is { IsAlive: true })
            {
                return Fail(WindowsGraphicsCaptureWindowSessionCode.StartFailed);
            }

            _startupCompleted = false;
            _stopSignal?.Dispose();
            stopSignal = new ManualResetEventSlim(false);
            _stopSignal = stopSignal;
            _snapshot = new(
                WindowsGraphicsCaptureWindowSessionCode.Starting,
                bindingSnapshot.WindowId,
                bindingSnapshot.Generation,
                0,
                0,
                0,
                0);
            _frameCount = 0;
            _lastTimestamp100Ns = 0;
            worker = new Thread(() => RunWorker(
                binding,
                bindingSnapshot,
                startup,
                frameConsumer,
                stopSignal))
            {
                IsBackground = true,
                Name = "gpautolive-wgc-capture",
            };
            _worker = worker;
        }

        try
        {
            worker.Start();
        }
        catch (Exception)
        {
            lock (_gate)
            {
                if (ReferenceEquals(_worker, worker))
                {
                    _worker = null;
                }

                if (ReferenceEquals(_stopSignal, stopSignal))
                {
                    _stopSignal = null;
                }
            }

            stopSignal.Dispose();
            SetSnapshot(WindowsGraphicsCaptureWindowSessionCode.StartFailed);
            startup.TrySetResult(new(false, WindowsGraphicsCaptureWindowSessionCode.StartFailed, Snapshot));
        }

        var budget = timeout.GetValueOrDefault(TimeSpan.FromMilliseconds(StartupTimeoutMilliseconds));
        if (budget <= TimeSpan.Zero || budget > TimeSpan.FromSeconds(30))
        {
            budget = TimeSpan.FromMilliseconds(StartupTimeoutMilliseconds);
        }

        var completed = await Task.WhenAny(
                startup.Task,
                Task.Delay(budget, cancellationToken))
            .ConfigureAwait(false);
        if (completed == startup.Task)
        {
            return await startup.Task.ConfigureAwait(false);
        }

        var code = cancellationToken.IsCancellationRequested
            ? WindowsGraphicsCaptureWindowSessionCode.Cancelled
            : WindowsGraphicsCaptureWindowSessionCode.StartFailed;
        await StopCoreAsync(code).ConfigureAwait(false);
        return new(false, code, Snapshot);
    }

    /// <summary>请求停止并在有限预算内等待 WGC 线程释放 WinRT/D3D11 资源。</summary>
    public Task StopAsync(CancellationToken cancellationToken = default) =>
        StopCoreAsync(WindowsGraphicsCaptureWindowSessionCode.Stopped, cancellationToken);

    public async ValueTask DisposeAsync()
    {
        lock (_gate)
        {
            _closed = true;
        }

        await StopCoreAsync(WindowsGraphicsCaptureWindowSessionCode.Closed).ConfigureAwait(false);
    }

    private async Task StopCoreAsync(
        WindowsGraphicsCaptureWindowSessionCode terminalCode,
        CancellationToken cancellationToken = default)
    {
        Thread? worker;
        ManualResetEventSlim? stopSignal;
        lock (_gate)
        {
            worker = _worker;
            stopSignal = _stopSignal;
            stopSignal?.Set();
        }

        if (worker is null || !worker.IsAlive)
        {
            SetSnapshot(terminalCode);
            return;
        }

        await Task.Run(
                () =>
                {
                    if (!worker.Join(StopJoinMilliseconds))
                    {
                        SetSnapshot(WindowsGraphicsCaptureWindowSessionCode.StartFailed);
                        return;
                    }

                    SetSnapshot(terminalCode);
                },
                CancellationToken.None)
            .ConfigureAwait(false);
    }

    private void RunWorker(
        WindowsVirtualCameraSurfaceBinding binding,
        WindowsVirtualCameraSurfaceBindingSnapshot bindingSnapshot,
        TaskCompletionSource<WindowsGraphicsCaptureWindowSessionResult> startup,
        Action<Direct3D11CaptureFrame, WindowsGraphicsCaptureD3D11Context>? frameConsumer,
        ManualResetEventSlim stopSignal)
    {
        var roInitialized = false;
        IntPtr nativeDevice = IntPtr.Zero;
        IntPtr nativeContext = IntPtr.Zero;
        IDirect3DDevice? directDevice = null;
        WindowsGraphicsCaptureD3D11Context? d3dContext = null;
        GraphicsCaptureItem? item = null;
        Direct3D11CaptureFramePool? framePool = null;
        GraphicsCaptureSession? captureSession = null;
        using var frameArrivedSignal = new AutoResetEvent(false);
        TypedEventHandler<Direct3D11CaptureFramePool, object>? frameArrivedHandler = null;
        var terminal = WindowsGraphicsCaptureWindowSessionCode.Stopped;

        try
        {
            if (!OperatingSystem.IsWindows())
            {
                terminal = WindowsGraphicsCaptureWindowSessionCode.NotWindows;
                CompleteStartup(startup, false, terminal);
                return;
            }

            if (!OperatingSystem.IsWindowsVersionAtLeast(10, 0, 17763))
            {
                terminal = WindowsGraphicsCaptureWindowSessionCode.UnsupportedVersion;
                CompleteStartup(startup, false, terminal);
                return;
            }

            var roResult = RoInitialize(1);
            if (roResult < 0)
            {
                terminal = WindowsGraphicsCaptureWindowSessionCode.RuntimeUnavailable;
                CompleteStartup(startup, false, terminal);
                return;
            }

            roInitialized = true;
            if (!binding.IsCurrent(bindingSnapshot.WindowId!.Value, bindingSnapshot.Generation))
            {
                terminal = WindowsGraphicsCaptureWindowSessionCode.InvalidBinding;
                CompleteStartup(startup, false, terminal);
                return;
            }

            if (!IsWindow(unchecked((nint)bindingSnapshot.WindowId.Value)))
            {
                terminal = WindowsGraphicsCaptureWindowSessionCode.InvalidHandle;
                CompleteStartup(startup, false, terminal);
                return;
            }

            if (!GraphicsCaptureSession.IsSupported())
            {
                terminal = WindowsGraphicsCaptureWindowSessionCode.RuntimeUnavailable;
                CompleteStartup(startup, false, terminal);
                return;
            }

            item = TryCreateCaptureItem(bindingSnapshot.WindowId.Value);
            if (item is null)
            {
                terminal = WindowsGraphicsCaptureWindowSessionCode.ItemUnavailable;
                CompleteStartup(startup, false, terminal);
                return;
            }

            var size = item.Size;
            if (size.Width <= 0 || size.Height <= 0)
            {
                terminal = WindowsGraphicsCaptureWindowSessionCode.ItemUnavailable;
                CompleteStartup(startup, false, terminal);
                return;
            }

            if (!TryCreateHardwareDevice(out nativeDevice, out nativeContext, out directDevice))
            {
                terminal = WindowsGraphicsCaptureWindowSessionCode.DeviceUnavailable;
                CompleteStartup(startup, false, terminal);
                return;
            }

            // 为同步 GPU 处理回调各持有一份 COM 引用，确保 frame pool 和回调使用同一设备。
            Marshal.AddRef(nativeDevice);
            Marshal.AddRef(nativeContext);
            d3dContext = new(
                new VorticeDevice(nativeDevice),
                new VorticeDeviceContext(nativeContext));

            framePool = Direct3D11CaptureFramePool.CreateFreeThreaded(
                directDevice,
                DirectXPixelFormat.B8G8R8A8UIntNormalized,
                FramePoolSize,
                size);
            captureSession = framePool.CreateCaptureSession(item);
            captureSession.IsCursorCaptureEnabled = false;
            if (OperatingSystem.IsWindowsVersionAtLeast(10, 0, 20348))
            {
                captureSession.IsBorderRequired = false;
            }
            frameArrivedHandler = (_, _) =>
            {
                try
                {
                    frameArrivedSignal.Set();
                }
                catch (ObjectDisposedException)
                {
                    // Stop/Dispose 与 WinRT 事件回调并发时，唤醒信号已无须再投递。
                }
            };
            framePool.FrameArrived += frameArrivedHandler;
            captureSession.StartCapture();

            SetSnapshot(
                WindowsGraphicsCaptureWindowSessionCode.Running,
                bindingSnapshot.WindowId,
                bindingSnapshot.Generation,
                size.Width,
                size.Height);
            CompleteStartup(startup, true, WindowsGraphicsCaptureWindowSessionCode.Running);

            var waitHandles = new WaitHandle[] { stopSignal.WaitHandle, frameArrivedSignal };
            while (true)
            {
                var signaled = WaitHandle.WaitAny(waitHandles, 100);
                if (signaled == 0)
                {
                    break;
                }

                // FrameArrived 是低延迟唤醒路径；超时也在同一捕获线程尝试排空，
                // 避免 WinRT 事件偶发未投递时 frame pool 永久积压而没有最终输出。
                if (!binding.IsCurrent(bindingSnapshot.WindowId.Value, bindingSnapshot.Generation))
                {
                    terminal = WindowsGraphicsCaptureWindowSessionCode.BindingInvalidated;
                    break;
                }

                while (true)
                {
                    try
                    {
                        using var frame = framePool.TryGetNextFrame();
                        if (frame is null)
                        {
                            break;
                        }

                        var contentSize = frame.ContentSize;
                        if (contentSize.Width != size.Width || contentSize.Height != size.Height)
                        {
                            terminal = WindowsGraphicsCaptureWindowSessionCode.FrameSizeChanged;
                            break;
                        }

                        if (frameConsumer is not null && d3dContext is not null)
                        {
                            frameConsumer(frame, d3dContext);
                        }

                        Interlocked.Increment(ref _frameCount);
                        Interlocked.Exchange(ref _lastTimestamp100Ns, frame.SystemRelativeTime.Ticks);
                    }
                    catch (COMException exception) when (exception.HResult == EBounds)
                    {
                        break;
                    }
                    catch (Exception)
                    {
                        terminal = WindowsGraphicsCaptureWindowSessionCode.StartFailed;
                        break;
                    }
                }

                if (terminal == WindowsGraphicsCaptureWindowSessionCode.FrameSizeChanged
                    || terminal == WindowsGraphicsCaptureWindowSessionCode.StartFailed)
                {
                    break;
                }
            }
        }
        catch (Exception)
        {
            terminal = WindowsGraphicsCaptureWindowSessionCode.StartFailed;
            CompleteStartup(startup, false, terminal);
        }
        finally
        {
            try
            {
                if (framePool is not null && frameArrivedHandler is not null)
                {
                    framePool.FrameArrived -= frameArrivedHandler;
                }
            }
            catch (Exception)
            {
                // 事件解绑失败不应跳过其余独立资源的释放。
            }

            // 每个资源独立释放；某个 WinRT/COM wrapper 释放失败不能短路后续清理。
            DisposeSafely(captureSession);
            DisposeSafely(framePool);
            // GraphicsCaptureItem 是 WinRT 投影对象，不实现 IDisposable；释放其依赖的
            // frame pool/session 后由投影层回收，不能把它强转为 IDisposable。
            DisposeSafely(directDevice);
            DisposeSafely(d3dContext);

            ReleaseComObject(nativeContext);
            ReleaseComObject(nativeDevice);
            if (roInitialized)
            {
                RoUninitialize();
            }

            var finalCode = terminal is WindowsGraphicsCaptureWindowSessionCode.Running
                or WindowsGraphicsCaptureWindowSessionCode.Stopped
                ? WindowsGraphicsCaptureWindowSessionCode.Stopped
                : terminal;
            SetSnapshot(finalCode);
            CompleteStartup(startup, false, finalCode);
            ManualResetEventSlim? signalToDispose = null;
            lock (_gate)
            {
                if (ReferenceEquals(_worker, Thread.CurrentThread))
                {
                    _worker = null;
                    if (ReferenceEquals(_stopSignal, stopSignal))
                    {
                        _stopSignal = null;
                        signalToDispose = stopSignal;
                    }
                }
            }

            signalToDispose?.Dispose();
        }
    }

    private void CompleteStartup(
        TaskCompletionSource<WindowsGraphicsCaptureWindowSessionResult> startup,
        bool success,
        WindowsGraphicsCaptureWindowSessionCode code)
    {
        SetSnapshot(code);
        lock (_gate)
        {
            if (_startupCompleted)
            {
                return;
            }

            _startupCompleted = true;
        }

        startup.TrySetResult(new(success, code, Snapshot));
    }

    private void SetSnapshot(
        WindowsGraphicsCaptureWindowSessionCode code,
        uint? windowId = null,
        ulong generation = 0,
        int width = 0,
        int height = 0)
    {
        lock (_gate)
        {
            var previous = _snapshot;
            _snapshot = new(
                code,
                windowId ?? previous.WindowId,
                generation == 0 ? previous.Generation : generation,
                width == 0 ? previous.Width : width,
                height == 0 ? previous.Height : height,
                unchecked((ulong)Math.Max(0, Interlocked.Read(ref _frameCount))),
                Interlocked.Read(ref _lastTimestamp100Ns));
        }
    }

    private WindowsGraphicsCaptureWindowSessionResult Fail(WindowsGraphicsCaptureWindowSessionCode code)
    {
        SetSnapshot(code);
        return new(false, code, Snapshot);
    }

    private static bool TryCreateHardwareDevice(
        out IntPtr nativeDevice,
        out IntPtr nativeContext,
        out IDirect3DDevice? directDevice)
    {
        nativeDevice = IntPtr.Zero;
        nativeContext = IntPtr.Zero;
        directDevice = null;
        try
        {
            using var factory = DXGI.CreateDXGIFactory1<IDXGIFactory1>();
            for (uint index = 0; index < MaxAdapterCount; index++)
            {
                IDXGIAdapter1? adapter = null;
                var enumResult = factory.EnumAdapters1(index, out adapter);
                if (enumResult.Failure || adapter is null)
                {
                    break;
                }

                try
                {
                    var description = adapter.Description1;
                    if (description.Flags.HasFlag(AdapterFlags.Software)
                        || description.Flags.HasFlag(AdapterFlags.Remote))
                    {
                        continue;
                    }

                    using var adapterBase = adapter.QueryInterface<IDXGIAdapter>();
                    if (TryCreateHardwareDeviceOnAdapter(
                            adapterBase.NativePointer,
                            out nativeDevice,
                            out nativeContext,
                            out directDevice))
                    {
                        return true;
                    }
                }
                catch (SharpGen.Runtime.SharpGenException)
                {
                    // 当前 adapter 不可用时继续尝试下一个真实硬件 adapter。
                }
                finally
                {
                    adapter.Dispose();
                }
            }
        }
        catch (SharpGen.Runtime.SharpGenException)
        {
            directDevice?.Dispose();
            directDevice = null;
            ReleaseComObject(nativeContext);
            ReleaseComObject(nativeDevice);
            nativeContext = IntPtr.Zero;
            nativeDevice = IntPtr.Zero;
            return false;
        }
        catch (COMException)
        {
            directDevice?.Dispose();
            directDevice = null;
            ReleaseComObject(nativeContext);
            ReleaseComObject(nativeDevice);
            nativeContext = IntPtr.Zero;
            nativeDevice = IntPtr.Zero;
            return false;
        }

        return false;
    }

    private static bool TryCreateHardwareDeviceOnAdapter(
        IntPtr adapter,
        out IntPtr nativeDevice,
        out IntPtr nativeContext,
        out IDirect3DDevice? directDevice)
    {
        nativeDevice = IntPtr.Zero;
        nativeContext = IntPtr.Zero;
        directDevice = null;
        var hresult = D3D11CreateDevice(
            adapter,
            UnknownDriverType,
            IntPtr.Zero,
            BgraSupport | VideoSupport,
            IntPtr.Zero,
            0,
            D3D11SdkVersion,
            out nativeDevice,
            out var featureLevel,
            out nativeContext);
        if (hresult < 0
            || nativeDevice == IntPtr.Zero
            || nativeContext == IntPtr.Zero
            || featureLevel < 0xb000)
        {
            ReleaseComObject(nativeContext);
            ReleaseComObject(nativeDevice);
            nativeContext = IntPtr.Zero;
            nativeDevice = IntPtr.Zero;
            return false;
        }

        try
        {
            var dxgiDeviceIid = DxgiDeviceIid;
            var queryResult = Marshal.QueryInterface(nativeDevice, in dxgiDeviceIid, out var dxgiDevice);
            if (queryResult < 0 || dxgiDevice == IntPtr.Zero)
            {
                return false;
            }

            try
            {
                var wrapResult = CreateDirect3D11DeviceFromDXGIDevice(dxgiDevice, out var inspectable);
                if (wrapResult < 0 || inspectable == IntPtr.Zero)
                {
                    return false;
                }

                try
                {
                    directDevice = WinRT.ComWrappersSupport.CreateRcwForComObject<IDirect3DDevice>(inspectable);
                    return directDevice is not null;
                }
                finally
                {
                    // CreateRcwForComObject creates its own managed COM reference.
                    ReleaseComObject(inspectable);
                }
            }
            finally
            {
                ReleaseComObject(dxgiDevice);
            }
        }
        catch (SharpGen.Runtime.SharpGenException)
        {
            return false;
        }
        catch (COMException)
        {
            return false;
        }
        finally
        {
            if (directDevice is null)
            {
                ReleaseComObject(nativeContext);
                ReleaseComObject(nativeDevice);
                nativeContext = IntPtr.Zero;
                nativeDevice = IntPtr.Zero;
            }
        }
    }

    private static void ReleaseComObject(IntPtr value)
    {
        if (value == IntPtr.Zero)
        {
            return;
        }

        try
        {
            Marshal.Release(value);
        }
        catch (Exception)
        {
            // 部分初始化失败时句柄可能不完整；释放失败不得破坏宿主退出。
        }
    }

    private static void DisposeSafely(IDisposable? resource)
    {
        try
        {
            resource?.Dispose();
        }
        catch (Exception)
        {
            // 资源释放失败只保留稳定终态，不把原生异常正文传播到 UI。
        }
    }

    private static GraphicsCaptureItem? TryCreateCaptureItem(ulong windowId)
    {
        // 与 Rust 参考实现保持同一 WinRT 工厂入口；它对 Win32 HWND 的语义最明确。
        var interopItem = TryCreateCaptureItemViaInterop(unchecked((nint)windowId));
        if (interopItem is not null)
        {
            return interopItem;
        }

        if (OperatingSystem.IsWindowsVersionAtLeast(10, 0, 20348))
        {
            try
            {
                var item = GraphicsCaptureItem.TryCreateFromWindowId(new global::Windows.UI.WindowId(windowId));
                if (item is not null)
                {
                    return item;
                }
            }
            catch (ArgumentException)
            {
                // 不支持该 HWND 的系统继续走 WinRT interop 兼容路径。
            }
            catch (COMException)
            {
                // 当前系统的静态入口不可用时继续走 WinRT interop。
            }
        }

        return null;
    }

    private static GraphicsCaptureItem? TryCreateCaptureItemViaInterop(nint windowHandle)
    {
        var itemIid = GraphicsCaptureItemIid;
        WinRT.IObjectReference? activationFactory = null;
        try
        {
            activationFactory = WinRT.ActivationFactory.Get(
                "Windows.Graphics.Capture.GraphicsCaptureItem");
            var interop = activationFactory.AsInterface<IGraphicsCaptureItemInterop>();
            var createResult = interop.CreateForWindow(windowHandle, ref itemIid, out var itemPointer);
            if (createResult < 0 || itemPointer == IntPtr.Zero)
            {
                return null;
            }

            try
            {
                return GraphicsCaptureItem.FromAbi(itemPointer);
            }
            finally
            {
                ReleaseComObject(itemPointer);
            }
        }
        catch (COMException)
        {
            return null;
        }
        finally
        {
            activationFactory?.Dispose();
        }
    }

    private static readonly Guid GraphicsCaptureItemIid =
        new("79c3f95b-31f7-4ec2-a464-632ef5d30760");

    [ComImport]
    [Guid("3628e81b-3cac-4c60-b7f4-23ce0e0c3356")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    [ComVisible(true)]
    private interface IGraphicsCaptureItemInterop
    {
        [PreserveSig]
        int CreateForWindow([In] IntPtr window, [In] ref Guid riid, out IntPtr result);

        [PreserveSig]
        int CreateForMonitor([In] IntPtr monitor, [In] ref Guid riid, out IntPtr result);
    }

    [DllImport("user32.dll", ExactSpelling = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool IsWindow(nint windowHandle);

    [DllImport("combase.dll", ExactSpelling = true)]
    private static extern int RoInitialize(uint initType);

    [DllImport("combase.dll", ExactSpelling = true)]
    private static extern void RoUninitialize();

    [DllImport("d3d11.dll", ExactSpelling = true)]
    private static extern int D3D11CreateDevice(
        IntPtr adapter,
        uint driverType,
        IntPtr software,
        uint flags,
        IntPtr featureLevels,
        uint featureLevelsCount,
        uint sdkVersion,
        out IntPtr device,
        out uint featureLevel,
        out IntPtr immediateContext);

    [DllImport("d3d11.dll", ExactSpelling = true)]
    private static extern int CreateDirect3D11DeviceFromDXGIDevice(
        IntPtr dxgiDevice,
        out IntPtr graphicsDevice);
}
