using GpAutoLive.Contracts;
using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>RTMP 最终 PCM 会话的稳定错误分类。</summary>
public enum WindowsRtmpAudioSessionFailureCode
{
    InvalidArguments,
    AlreadyRunning,
    StartFailed,
    DecodeFailed,
    PumpFailed,
    Cancelled,
    StopTimedOut,
    Closed,
}

/// <summary>不包含媒体路径、地址或音频正文的 RTMP PCM 会话错误。</summary>
public sealed record WindowsRtmpAudioSessionError(
    WindowsRtmpAudioSessionFailureCode Code,
    string Message,
    bool Retryable = false);

/// <summary>RTMP PCM 会话脱敏快照。</summary>
public sealed record WindowsRtmpAudioSessionSnapshot(
    bool IsRunning,
    ulong ProducedFrames,
    ulong ForwardedFrames,
    string? ErrorCode,
    string? Error);

/// <summary>RTMP PCM 会话操作结果。</summary>
public sealed record WindowsRtmpAudioSessionResult(
    bool IsSuccess,
    WindowsRtmpAudioSessionSnapshot Snapshot,
    WindowsRtmpAudioSessionError? Error = null);

/// <summary>
/// 把已探测媒体的声音解码到 FinalPcmBus，并由唯一 RTMP 分流泵写入 FFmpeg stdin。
/// 它不捕获桌面，不读取第二份源音频，也不创建无界队列；视频轨道仍由 RTMP 宿主直接读取源文件。
/// 未提供共享最终 PCM 总线时，audioEffects 直接进入本会话的 FFmpeg PCM 解码计划；
/// videoEffects 交给同一 RTMP 宿主的受限视频滤镜桥接。
/// </summary>
public sealed class WindowsRtmpAudioSession : IAsyncDisposable
{
    private static readonly TimeSpan StopTimeout = TimeSpan.FromSeconds(2);
    private static readonly TimeSpan InitialPcmForwardTimeout = TimeSpan.FromSeconds(3);
    private static readonly TimeSpan InitialPcmForwardPollInterval = TimeSpan.FromMilliseconds(10);
    private readonly object _gate = new();
    private readonly SemaphoreSlim _lifecycle = new(1, 1);
    private readonly WindowsRtmpOutputManager _manager;
    private WindowsFfmpegPcmDecoder? _decoder;
    private WindowsFfmpegPcmDecoder? _overlayDecoder;
    private WindowsRtmpFinalPcmPump? _pump;
    private FinalPcmBus? _bus;
    private AudioPcmMixingOutputSource? _mixingOutputSource;
    private CancellationTokenSource? _sessionCancellation;
    private CancellationTokenSource? _overlayCancellation;
    private Task? _producerTask;
    private Task? _pumpTask;
    private Task? _overlayTask;
    private Func<AudioPcmMixPolicy>? _baseMixPolicyProvider;
    private Func<FinalPcmBus?>? _sharedFinalPcmBusProvider;
    private bool _ownsBus;
    private bool _disposed;
    private bool _disposeStarted;
    private bool _running;
    private ulong _producedFrames;
    private ulong _forwardedFrames;
    private WindowsRtmpAudioSessionError? _lastError;

    public WindowsRtmpAudioSession(WindowsRtmpOutputManager manager)
    {
        _manager = manager ?? throw new ArgumentNullException(nameof(manager));
    }

    public WindowsRtmpAudioSessionSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return new(
                    _running,
                    _producedFrames,
                    _pump?.Snapshot.ForwardedFrames ?? _forwardedFrames,
                    _lastError?.Code.ToString(),
                    _lastError?.Message);
            }
        }
    }

    /// <summary>当前 RTMP 会话的插话解码完成任务；未启动插话时为已完成任务。</summary>
    public Task InterludeCompletion
    {
        get
        {
            lock (_gate)
            {
                return _overlayTask ?? Task.CompletedTask;
            }
        }
    }

    public async Task<WindowsRtmpAudioSessionResult> StartAsync(
        RtmpOutputConfig? config,
        SourceMediaDto? source,
        string? ffmpegPath,
        RtmpSourceIdentity? sourceIdentity,
        string? preferredEncoder = null,
        CancellationToken cancellationToken = default,
        Func<AudioPcmMixPolicy>? baseMixPolicyProvider = null,
        AudioPcmMixEnvelopeOptions? mixEnvelopeOptions = null,
        FinalPcmBus? sharedFinalPcmBus = null,
        IAudioPcmOutputSource? sharedRtmpOutputSource = null,
        IAudioPcmOutputSource? sharedRtmpOverlayOutputSource = null,
        Func<FinalPcmBus?>? sharedFinalPcmBusProvider = null,
        AudioEffectParams? audioEffects = null,
        MpvVideoEffectSnapshot? videoEffects = null)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsRtmpAudioSessionFailureCode.Cancelled, "RTMP 声音会话启动已取消。", true);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure(WindowsRtmpAudioSessionFailureCode.Closed, "RTMP 声音会话已关闭。");
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsRtmpAudioSessionFailureCode.Cancelled, "RTMP 声音会话启动已取消。", true);
        }

        try
        {
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsRtmpAudioSessionFailureCode.Closed, "RTMP 声音会话已关闭。");
                }

                if (_running)
                {
                    return Failure(WindowsRtmpAudioSessionFailureCode.AlreadyRunning, "RTMP 声音会话已经在运行。", false);
                }

                _lastError = null;
                _producedFrames = 0;
                _forwardedFrames = 0;
            }

            if (config is null || !config.AudioEnabled)
            {
                return Failure(WindowsRtmpAudioSessionFailureCode.InvalidArguments, "RTMP 声音会话必须启用声音轨道。", false);
            }

            if (source is null
                || source.AudioChannelCount is null or 0
                || string.IsNullOrWhiteSpace(source.AudioCodecName))
            {
                return Failure(WindowsRtmpAudioSessionFailureCode.InvalidArguments, "当前媒体没有可用声音轨道。", false);
            }

            var outputChannels = sharedFinalPcmBus?.Channels ?? FinalPcmBus.DefaultChannels;
            if (!FfmpegPcmDecodePlanBuilder.TryCreate(
                    ffmpegPath,
                    source,
                    FfmpegPcmDecodePlanBuilder.DefaultSampleRateHz,
                    outputChannels,
                    out var decodePlan,
                    out var decodeError,
                    audioEffects)
                || decodePlan is null)
            {
                return Failure(
                    WindowsRtmpAudioSessionFailureCode.InvalidArguments,
                    decodeError?.Message ?? "RTMP 声音解码计划无效。",
                    decodeError?.Retryable == true);
            }

            if (sharedFinalPcmBus is not null
                && sharedFinalPcmBus.Channels != decodePlan.Channels)
            {
                return Failure(
                    WindowsRtmpAudioSessionFailureCode.InvalidArguments,
                    "共享最终 PCM 总线声道数与 RTMP 音频计划不一致。",
                    false);
            }

            if ((sharedRtmpOutputSource is not null
                    && sharedRtmpOutputSource.Channels != decodePlan.Channels)
                || (sharedRtmpOverlayOutputSource is not null
                    && sharedRtmpOverlayOutputSource.Channels != decodePlan.Channels))
            {
                return Failure(
                    WindowsRtmpAudioSessionFailureCode.InvalidArguments,
                    "共享 RTMP PCM 输出源声道数与 RTMP 音频计划不一致。",
                    false);
            }

            if ((sharedRtmpOutputSource is not null || sharedRtmpOverlayOutputSource is not null)
                && sharedFinalPcmBus is null)
            {
                return Failure(
                    WindowsRtmpAudioSessionFailureCode.InvalidArguments,
                    "共享 RTMP PCM 输出源必须同时提供最终 PCM 总线。",
                    false);
            }

            var started = await _manager.StartAsync(
                    config,
                    source,
                    ffmpegPath,
                    preferredEncoder: preferredEncoder,
                    sourceIdentity: sourceIdentity,
                    videoEffects: videoEffects,
                    cancellationToken: cancellationToken)
                .ConfigureAwait(false);
            if (!started.IsSuccess)
            {
                return Failure(
                    WindowsRtmpAudioSessionFailureCode.StartFailed,
                    started.Error?.Message ?? "RTMP 声音宿主启动失败。",
                    started.Error?.Retryable == true);
            }

            var ownsBus = sharedFinalPcmBus is null;
            var bus = sharedFinalPcmBus
                ?? new FinalPcmBus(capacityFrames: 48_000, channels: FinalPcmBus.DefaultChannels);
            bus.SetRtmpConsumerAttached(true);
            var decoder = ownsBus ? new WindowsFfmpegPcmDecoder() : null;
            var mixingOutputSource = new AudioPcmMixingOutputSource(
                sharedRtmpOutputSource ?? new AudioPcmRingBufferOutputSource(bus.RtmpBuffer),
                sharedRtmpOverlayOutputSource ?? new AudioPcmRingBufferOutputSource(bus.RtmpOverlayBuffer),
                outputChannels,
                policyProvider: baseMixPolicyProvider,
                envelopeOptions: mixEnvelopeOptions);
            var pump = new WindowsRtmpFinalPcmPump(
                mixingOutputSource,
                outputChannels);
            var sessionCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            var producerStartup = new TaskCompletionSource<WindowsRtmpAudioSessionError?>(
                TaskCreationOptions.RunContinuationsAsynchronously);
            var pumpStartup = new TaskCompletionSource<WindowsRtmpAudioSessionError?>(
                TaskCreationOptions.RunContinuationsAsynchronously);
            Task producerTask;
            Task pumpTask;
            lock (_gate)
            {
                _bus = bus;
                _decoder = decoder;
                _pump = pump;
                _mixingOutputSource = mixingOutputSource;
                _sessionCancellation = sessionCancellation;
                _ownsBus = ownsBus;
                _sharedFinalPcmBusProvider = sharedFinalPcmBusProvider;
                _running = true;
                // RTMP 分流泵在固定输出缓冲内应用基础轨/插话策略；解码线程不再重复 duck。
                _baseMixPolicyProvider = null;
                producerTask = _producerTask = decoder is null
                    ? Task.CompletedTask
                    : ProduceAsync(decodePlan, bus, decoder, sessionCancellation, producerStartup);
                pumpTask = _pumpTask = RunPumpAsync(pump, sessionCancellation, pumpStartup);
                if (decoder is null)
                {
                    producerStartup.TrySetResult(null);
                }
            }

            var startupError = producerStartup.Task.IsCompletedSuccessfully
                ? await producerStartup.Task.ConfigureAwait(false)
                : null;
            startupError ??= pumpStartup.Task.IsCompletedSuccessfully
                ? await pumpStartup.Task.ConfigureAwait(false)
                : null;
            if (startupError is not null)
            {
                await WaitTaskAsync(producerTask).ConfigureAwait(false);
                await WaitTaskAsync(pumpTask).ConfigureAwait(false);
                return Failure(startupError.Code, startupError.Message, startupError.Retryable);
            }

            var initialForwardError = await WaitForInitialPcmForwardAsync(
                    pump,
                    cancellationToken)
                .ConfigureAwait(false);
            if (initialForwardError is not null)
            {
                var stopped = await StopCoreAsync().ConfigureAwait(false);
                return stopped.IsSuccess
                    ? Failure(
                        initialForwardError.Code,
                        initialForwardError.Message,
                        initialForwardError.Retryable)
                    : Failure(
                        WindowsRtmpAudioSessionFailureCode.StopTimedOut,
                        stopped.Error?.Message ?? "RTMP 首帧失败后的会话收尾未完成。",
                        retryable: true);
            }

            return Success();
        }
        catch (OperationCanceledException)
        {
            await StopCoreAsync().ConfigureAwait(false);
            return Failure(WindowsRtmpAudioSessionFailureCode.Cancelled, "RTMP 声音会话启动已取消。", true);
        }
        catch (Exception)
        {
            await StopCoreAsync().ConfigureAwait(false);
            return Failure(WindowsRtmpAudioSessionFailureCode.StartFailed, "RTMP 声音会话启动失败。", true);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>在当前 RTMP 声音会话上启动一次有界插话解码，共享同一 PCM 分流泵。</summary>
    public async Task<WindowsRtmpAudioSessionResult> StartInterludeAsync(
        FfmpegPcmDecodePlan? plan,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure(WindowsRtmpAudioSessionFailureCode.Cancelled, "RTMP 插话解码启动已取消。", true);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure(WindowsRtmpAudioSessionFailureCode.Closed, "RTMP 声音会话已关闭。");
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsRtmpAudioSessionFailureCode.Cancelled, "RTMP 插话解码启动已取消。", true);
        }

        try
        {
            CancellationTokenSource? sessionCancellation;
            FinalPcmBus? bus;
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure(WindowsRtmpAudioSessionFailureCode.Closed, "RTMP 声音会话已关闭。");
                }

                if (!_running || _sessionCancellation is null || _bus is null || _mixingOutputSource is null)
                {
                    return Failure(WindowsRtmpAudioSessionFailureCode.InvalidArguments, "当前没有可承载插话的 RTMP 声音会话。");
                }

                if (_overlayTask is not null)
                {
                    return Failure(WindowsRtmpAudioSessionFailureCode.AlreadyRunning, "RTMP 插话解码已经在运行。", false);
                }

                sessionCancellation = _sessionCancellation;
                bus = _sharedFinalPcmBusProvider?.Invoke() ?? _bus;
            }

            if (bus is null || plan is null || plan.Channels != bus.Channels)
            {
                return Failure(WindowsRtmpAudioSessionFailureCode.InvalidArguments, "RTMP 插话解码计划或声道数无效。", false);
            }

            var overlayCancellation = CancellationTokenSource.CreateLinkedTokenSource(
                sessionCancellation.Token,
                cancellationToken);
            var decoder = new WindowsFfmpegPcmDecoder();
            var staleSession = false;
            lock (_gate)
            {
                if (!_running
                    || _sessionCancellation is null
                    || _bus is null
                    || _overlayTask is not null)
                {
                    staleSession = true;
                }
                else
                {
                    _overlayDecoder = decoder;
                    _overlayCancellation = overlayCancellation;
                    _overlayTask = RunInterludeAsync(
                        plan,
                        bus,
                        decoder,
                        overlayCancellation);
                }
            }

            if (staleSession)
            {
                overlayCancellation.Dispose();
                await decoder.DisposeAsync().ConfigureAwait(false);
                return Failure(WindowsRtmpAudioSessionFailureCode.InvalidArguments, "RTMP 声音会话已变化，插话启动被拒绝。", true);
            }

            return Success();
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>停止当前 RTMP 插话并在有界预算内 Join；主推流保持不变。</summary>
    public async Task<WindowsRtmpAudioSessionResult> StopInterludeAsync()
    {
        try
        {
            await _lifecycle.WaitAsync().ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Success();
        }

        try
        {
            return await StopInterludeCoreAsync().ConfigureAwait(false);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    public async Task<WindowsRtmpAudioSessionResult> StopAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure(WindowsRtmpAudioSessionFailureCode.Closed, "RTMP 声音会话已关闭。");
        }
        catch (OperationCanceledException)
        {
            return Failure(WindowsRtmpAudioSessionFailureCode.Cancelled, "RTMP 声音会话停止已取消。", true);
        }

        try
        {
            return await StopCoreAsync().ConfigureAwait(false);
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    public async ValueTask DisposeAsync()
    {
        lock (_gate)
        {
            if (_disposeStarted)
            {
                return;
            }

            _disposeStarted = true;
            _disposed = true;
        }

        var stopped = await StopAsync(CancellationToken.None).ConfigureAwait(false);
        if (!stopped.IsSuccess)
        {
            // 停止超时时保留生命周期信号和资源引用，允许调用方再次 Stop/Dispose
            // 完成底层宿主回收；不能把未 Join 的会话伪装成已释放。
            lock (_gate)
            {
                _disposeStarted = false;
            }

            return;
        }

        _lifecycle.Dispose();
        GC.SuppressFinalize(this);
    }

    private async Task ProduceAsync(
        FfmpegPcmDecodePlan plan,
        FinalPcmBus bus,
        WindowsFfmpegPcmDecoder decoder,
        CancellationTokenSource sessionCancellation,
        TaskCompletionSource<WindowsRtmpAudioSessionError?> startup)
    {
        var cancellationToken = sessionCancellation.Token;
        try
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                var decodeTask = decoder.DecodeAsync(
                        plan,
                        destination: null,
                        cancellationToken,
                        finalPcmBus: bus,
                        baseMixPolicyProvider: _baseMixPolicyProvider)
                    ;
                if (!decodeTask.IsCompleted)
                {
                    startup.TrySetResult(null);
                }

                var result = await decodeTask.ConfigureAwait(false);
                if (!result.IsSuccess)
                {
                    var error = new WindowsRtmpAudioSessionError(
                        WindowsRtmpAudioSessionFailureCode.DecodeFailed,
                        result.Error?.Message ?? "RTMP 声音解码失败。",
                        result.Error?.Retryable == true);
                    startup.TrySetResult(error);
                    if (!cancellationToken.IsCancellationRequested)
                    {
                        SetError(
                            error.Code,
                            error.Message,
                            error.Retryable);
                        lock (_gate)
                        {
                            _running = false;
                        }
                        sessionCancellation.Cancel();
                        await _manager.StopAsync(CancellationToken.None).ConfigureAwait(false);
                    }

                    return;
                }

                startup.TrySetResult(null);
                AddProducedFrames(result.Snapshot.DecodedFrames);
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch (Exception)
        {
            var error = new WindowsRtmpAudioSessionError(
                WindowsRtmpAudioSessionFailureCode.DecodeFailed,
                "RTMP 声音解码失败。",
                true);
            startup.TrySetResult(error);
            if (!cancellationToken.IsCancellationRequested)
            {
                SetError(error.Code, error.Message, error.Retryable);
                lock (_gate)
                {
                    _running = false;
                }
                sessionCancellation.Cancel();
                await _manager.StopAsync(CancellationToken.None).ConfigureAwait(false);
            }
        }
    }

    private async Task RunPumpAsync(
        WindowsRtmpFinalPcmPump pump,
        CancellationTokenSource sessionCancellation,
        TaskCompletionSource<WindowsRtmpAudioSessionError?> startup)
    {
        WindowsRtmpPcmPumpResult result;
        try
        {
            var runTask = pump.RunAsync(_manager, sessionCancellation.Token);
            if (!runTask.IsCompleted)
            {
                startup.TrySetResult(null);
            }

            result = await runTask.ConfigureAwait(false);
        }
        catch (Exception)
        {
            result = new(
                false,
                pump.Snapshot,
                new WindowsRtmpPcmPumpError(
                    WindowsRtmpPcmPumpFailureCode.WriteFailed,
                    "RTMP PCM 分流任务异常退出。",
                    Retryable: true));
        }

        var error = MapPumpFailure(result, sessionCancellation.IsCancellationRequested);
        startup.TrySetResult(error);
        if (error is null)
        {
            return;
        }

        lock (_gate)
        {
            _running = false;
            _lastError = error;
        }
        sessionCancellation.Cancel();
        await _manager.StopAsync(CancellationToken.None).ConfigureAwait(false);
    }

    internal static WindowsRtmpAudioSessionError? MapPumpFailure(
        WindowsRtmpPcmPumpResult result,
        bool sessionCancellationRequested)
    {
        if (result.IsSuccess || sessionCancellationRequested)
        {
            return null;
        }

        return new(
            WindowsRtmpAudioSessionFailureCode.PumpFailed,
            result.Error?.Message ?? "RTMP PCM 分流失败。",
            result.Error?.Retryable ?? true);
    }

    private async Task RunInterludeAsync(
        FfmpegPcmDecodePlan plan,
        FinalPcmBus bus,
        WindowsFfmpegPcmDecoder decoder,
        CancellationTokenSource cancellation)
    {
        try
        {
            _ = await decoder.DecodeAsync(
                    plan,
                    destination: null,
                    cancellation.Token,
                    finalPcmBus: bus,
                    baseMixPolicyProvider: null,
                    finalPcmOverlay: true)
                .ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellation.IsCancellationRequested)
        {
        }
        finally
        {
            await decoder.DisposeAsync().ConfigureAwait(false);
            lock (_gate)
            {
                if (ReferenceEquals(_overlayDecoder, decoder))
                {
                    _overlayDecoder = null;
                    _overlayCancellation = null;
                    _overlayTask = null;
                }
            }

            cancellation.Dispose();
        }
    }

    private async Task<WindowsRtmpAudioSessionResult> StopCoreAsync()
    {
        CancellationTokenSource? sessionCancellation;
        WindowsFfmpegPcmDecoder? decoder;
        WindowsRtmpFinalPcmPump? pump;
        FinalPcmBus? bus;
        Task? producerTask;
        Task? pumpTask;
        bool ownsBus;
        lock (_gate)
        {
            sessionCancellation = _sessionCancellation;
            decoder = _decoder;
            pump = _pump;
            bus = _bus;
            producerTask = _producerTask;
            pumpTask = _pumpTask;
            ownsBus = _ownsBus;
            _running = false;
        }

        if (sessionCancellation is null)
        {
            // 已进入停止临界区后必须完成宿主回收；调用方取消只影响进入临界区，不能留下 FFmpeg。
            await _manager.StopAsync(CancellationToken.None).ConfigureAwait(false);
            return Success();
        }

        sessionCancellation.Cancel();
        var interludeStopped = await StopInterludeCoreAsync().ConfigureAwait(false);
        if (ownsBus)
        {
            bus?.Close();
        }
        else
        {
            bus?.SetRtmpConsumerAttached(false);
        }
        decoder?.Stop();
        // 两个任务都必须独立等待；不能因为生产者超时就短路，留下仍在访问总线的分流任务。
        var producerJoined = await WaitTaskAsync(producerTask).ConfigureAwait(false);
        var pumpJoined = await WaitTaskAsync(pumpTask).ConfigureAwait(false);
        var joined = interludeStopped.IsSuccess && producerJoined && pumpJoined;
        WindowsRtmpResult stopped;
        if (!joined)
        {
            // 先停止宿主，解除可能阻塞在 stdin 的 Pump，再给所有任务一次有界 Join
            // 机会；未 Join 前保留总线、解码器和任务引用，交由下一次 Stop 重试。
            stopped = await _manager.StopAsync(CancellationToken.None).ConfigureAwait(false);
            producerJoined |= await WaitTaskAsync(producerTask).ConfigureAwait(false);
            pumpJoined |= await WaitTaskAsync(pumpTask).ConfigureAwait(false);
            joined = interludeStopped.IsSuccess && producerJoined && pumpJoined;
        }
        else
        {
            stopped = await _manager.StopAsync(CancellationToken.None).ConfigureAwait(false);
        }

        if (!joined)
        {
            return Failure(WindowsRtmpAudioSessionFailureCode.StopTimedOut, "RTMP 声音会话未能在停止预算内结束。", true);
        }

        if (!stopped.IsSuccess)
        {
            // 即使 Producer/Pump 已完成，也不能在 RTMP 进程尚未确认停止时清空
            // 会话资源；保留所有者，让下一次 StopAsync 重试底层宿主回收。
            return Failure(
                WindowsRtmpAudioSessionFailureCode.StopTimedOut,
                stopped.Error?.Message ?? "RTMP 宿主停止失败。",
                true);
        }

        pump?.Dispose();
        if (decoder is not null)
        {
            await decoder.DisposeAsync().ConfigureAwait(false);
        }
        // 同上，停止阶段使用内部有界预算，避免窗口取消令牌让宿主和总线所有权悬空。
        if (ownsBus)
        {
            bus?.Dispose();
        }
        sessionCancellation.Dispose();
        lock (_gate)
        {
            _decoder = null;
            _overlayDecoder = null;
            _pump = null;
            _bus = null;
            _ownsBus = false;
            _mixingOutputSource = null;
            _sessionCancellation = null;
            _producerTask = null;
            _pumpTask = null;
            _overlayCancellation = null;
            _overlayTask = null;
            _baseMixPolicyProvider = null;
            _sharedFinalPcmBusProvider = null;
        }

        return Success();
    }

    private async Task<WindowsRtmpAudioSessionResult> StopInterludeCoreAsync()
    {
        CancellationTokenSource? cancellation;
        WindowsFfmpegPcmDecoder? decoder;
        Task? task;
        lock (_gate)
        {
            cancellation = _overlayCancellation;
            decoder = _overlayDecoder;
            task = _overlayTask;
        }

        if (cancellation is null || task is null)
        {
            return Success();
        }

        cancellation.Cancel();
        decoder?.Stop();
        try
        {
            await task.WaitAsync(StopTimeout).ConfigureAwait(false);
            return Success();
        }
        catch (TimeoutException)
        {
            return Failure(WindowsRtmpAudioSessionFailureCode.StopTimedOut, "RTMP 插话解码未能在停止预算内结束。", true);
        }
    }

    private static async Task<bool> WaitTaskAsync(Task? task)
    {
        if (task is null)
        {
            return true;
        }

        try
        {
            await task.WaitAsync(StopTimeout).ConfigureAwait(false);
            return true;
        }
        catch (TimeoutException)
        {
            return false;
        }
        catch (Exception)
        {
            // 任务已完成；异常不会留下运行中的线程，继续回收其拥有的资源。
            return true;
        }
    }

    private static async Task<WindowsRtmpAudioSessionError?> WaitForInitialPcmForwardAsync(
        WindowsRtmpFinalPcmPump pump,
        CancellationToken cancellationToken)
    {
        var deadline = DateTime.UtcNow + InitialPcmForwardTimeout;
        while (DateTime.UtcNow < deadline)
        {
            var snapshot = pump.Snapshot;
            if (snapshot.ForwardedFrames > 0)
            {
                return null;
            }

            if (snapshot.ErrorCode is not null)
            {
                return new(
                    WindowsRtmpAudioSessionFailureCode.PumpFailed,
                    snapshot.Error ?? "RTMP PCM 分流泵在首帧前失败。",
                    Retryable: true);
            }

            await Task.Delay(InitialPcmForwardPollInterval, cancellationToken)
                .ConfigureAwait(false);
        }

        return new(
            WindowsRtmpAudioSessionFailureCode.PumpFailed,
            "RTMP 声音会话在启动预算内未转发首批最终 PCM。",
            Retryable: true);
    }

    private void AddProducedFrames(ulong frames)
    {
        lock (_gate)
        {
            _producedFrames = ulong.MaxValue - frames < _producedFrames
                ? ulong.MaxValue
                : _producedFrames + frames;
        }
    }

    private void SetError(
        WindowsRtmpAudioSessionFailureCode code,
        string message,
        bool retryable)
    {
        lock (_gate)
        {
            _lastError = new(code, message, retryable);
        }
    }

    private WindowsRtmpAudioSessionResult Success() => new(true, Snapshot);

    private WindowsRtmpAudioSessionResult Failure(
        WindowsRtmpAudioSessionFailureCode code,
        string message,
        bool retryable = false)
    {
        var error = new WindowsRtmpAudioSessionError(code, message, retryable);
        lock (_gate)
        {
            _lastError = error;
            return new(false, new(
                _running,
                _producedFrames,
                _pump?.Snapshot.ForwardedFrames ?? _forwardedFrames,
                error.Code.ToString(),
                error.Message), error);
        }
    }
}
