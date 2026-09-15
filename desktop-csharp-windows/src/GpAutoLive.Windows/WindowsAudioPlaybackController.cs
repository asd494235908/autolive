using GpAutoLive.Media;

namespace GpAutoLive.Windows;

/// <summary>本机音频播放控制器的稳定状态。</summary>
public enum WindowsAudioPlaybackState
{
    Idle,
    Starting,
    Playing,
    Paused,
    Stopping,
    Completed,
    Failed,
    Closed,
}

/// <summary>不包含音频正文或路径的本机音频播放快照。</summary>
public sealed record WindowsAudioPlaybackSnapshot(
    WindowsAudioPlaybackState State,
    WindowsPortAudioOutputSnapshot? Output,
    WindowsFfmpegPcmDecoderSnapshot? Decoder,
    string? ErrorCode,
    string? Error)
{
    public WindowsAudibleAudioClockSnapshot? AudibleClock { get; init; }
}

/// <summary>本机音频播放启动结果。</summary>
public sealed record WindowsAudioPlaybackError(
    string Code,
    string Message,
    bool Retryable = false);

public sealed record WindowsAudioPlaybackResult(
    bool IsSuccess,
    WindowsAudioPlaybackSnapshot Snapshot,
    WindowsAudioPlaybackError? Error = null);

/// <summary>
/// 将单次 FFmpeg PCM 解码、固定容量环缓（或最终 PCM 双消费者总线）和 PortAudio 输出流
/// 组合为一个可取消的本机播放会话。解码结束后关闭音频总线并停止输出，不保留完整音频文件；
/// 循环由有界的重新解码实现。
/// </summary>
public sealed partial class WindowsAudioPlaybackController : IAsyncDisposable
{
    private static readonly TimeSpan StopTimeout = TimeSpan.FromSeconds(2);
    private static readonly TimeSpan DrainTimeout = TimeSpan.FromSeconds(5);
    private static readonly TimeSpan CandidateTransitionTimeout = TimeSpan.FromSeconds(5);
    private static readonly TimeSpan CandidateReadyTimeout = TimeSpan.FromSeconds(2);
    private static readonly TimeSpan InitialPcmReadyTimeout = TimeSpan.FromSeconds(3);
    private static readonly TimeSpan CandidatePollInterval = TimeSpan.FromMilliseconds(10);
    private const int CandidateMinimumReadyMs = 250;
    private const ulong SustainedXrunMinimumCallbacks = 1_024;
    private const ulong SustainedXrunMinimumEvents = 32;
    private const int MaxSustainedXrunRecoveries = 3;
    private readonly object _gate = new();
    private readonly SemaphoreSlim _lifecycle = new(1, 1);
    private readonly int _capacityFrames;
    private readonly WindowsPortAudioOutputRecovery _outputRecovery;
    private AudioPcmRingBuffer? _buffer;
    private WindowsPortAudioOutputStream? _output;
    private WindowsAudibleAudioClock? _audibleAudioClock;
    private WindowsFfmpegPcmDecoder? _decoder;
    private WindowsFfmpegPcmDecoder? _overlayDecoder;
    private FinalPcmBus? _finalPcmBus;
    private FinalPcmBusTrackSwitch? _finalPcmBusTrackSwitch;
    private WindowsAudioPauseGate? _pauseGate;
    private AudioPcmRingBuffer? _overlayBuffer;
    private AudioPcmMixingOutputSource? _mixingOutputSource;
    private Func<AudioPcmMixPolicy>? _baseMixPolicyProvider;
    private CancellationTokenSource? _sessionCancellation;
    private CancellationTokenSource? _overlayCancellation;
    private Task? _sessionTask;
    private Task? _overlayTask;
    private PreparedAudioCandidate? _preparedCandidate;
    private TaskCompletionSource<WindowsFfmpegPcmDecoderResult>? _candidateCompletion;
    private ulong _candidateSequence;
    private WindowsAudioPlaybackState _state = WindowsAudioPlaybackState.Idle;
    private WindowsAudioPlaybackError? _lastError;
    private bool _disposed;
    private Task? _disposeTask;

    private sealed class PreparedAudioCandidate
    {
        public required ulong CandidateId { get; init; }

        public required FfmpegPcmDecodePlan Plan { get; init; }

        public required FinalPcmBus Bus { get; init; }

        public required WindowsFfmpegPcmDecoder Decoder { get; init; }

        public required CancellationTokenSource Cancellation { get; init; }

        public required TaskCompletionSource<WindowsFfmpegPcmDecoderResult> Completion { get; init; }

        public required TaskCompletionSource<bool> CommitRequested { get; init; }

        public required TaskCompletionSource<bool> Activated { get; init; }

        public Task<WindowsFfmpegPcmDecoderResult>? DecodeTask { get; set; }

        public int CleanupStarted;
    }

    public WindowsAudioPlaybackController(
        int capacityFrames = 48_000,
        WindowsPortAudioOutputRecovery? outputRecovery = null)
    {
        if (capacityFrames is < 256 or > 480_000)
        {
            throw new ArgumentOutOfRangeException(nameof(capacityFrames), "音频播放环缓容量必须在 256 到 480000 帧内。");
        }

        _capacityFrames = capacityFrames;
        _outputRecovery = outputRecovery ?? new WindowsPortAudioOutputRecovery();
    }

    public WindowsAudioPlaybackSnapshot Snapshot
    {
        get
        {
            lock (_gate)
            {
                return CreateSnapshotUnsafe();
            }
        }
    }

    /// <summary>会话完成任务；未启动时为已完成任务。</summary>
    public Task Completion
    {
        get
        {
            lock (_gate)
            {
                return _sessionTask ?? Task.CompletedTask;
            }
        }
    }

    /// <summary>当前插话解码完成任务；未启动插话时为已完成任务。</summary>
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

    /// <summary>
    /// 当前会话使用的最终 PCM 总线；调用方只可将其交给另一个消费者，不能关闭或释放。
    /// </summary>
    public FinalPcmBus? ActiveFinalPcmBus
    {
        get
        {
            lock (_gate)
            {
                return _finalPcmBusTrackSwitch?.ActiveBus ?? _finalPcmBus;
            }
        }
    }

    /// <summary>当前会话的稳定 RTMP PCM 输出源；若无会话则为 null。</summary>
    public IAudioPcmOutputSource? ActiveFinalPcmRtmpOutputSource
    {
        get
        {
            lock (_gate)
            {
                return _finalPcmBusTrackSwitch?.RtmpSource
                    ?? (_finalPcmBus is null
                        ? null
                        : new AudioPcmRingBufferOutputSource(_finalPcmBus.RtmpBuffer));
            }
        }
    }

    /// <summary>当前会话的稳定 RTMP 插话 PCM 输出源；若无会话则为 null。</summary>
    public IAudioPcmOutputSource? ActiveFinalPcmRtmpOverlayOutputSource
    {
        get
        {
            lock (_gate)
            {
                return _finalPcmBusTrackSwitch?.RtmpOverlaySource
                    ?? (_finalPcmBus is null
                        ? null
                        : new AudioPcmRingBufferOutputSource(_finalPcmBus.RtmpOverlayBuffer));
            }
        }
    }

    /// <summary>当前音频候选是否已预载一个 N+1 候选。</summary>
    public bool HasPreparedNext
    {
        get
        {
            lock (_gate)
            {
                return _preparedCandidate is not null;
            }
        }
    }

    /// <summary>当前候选到达有限 EOF 的任务；不会等待下一候选。</summary>
    public Task<WindowsFfmpegPcmDecoderResult>? CandidateCompletion
    {
        get
        {
            lock (_gate)
            {
                return _candidateCompletion?.Task;
            }
        }
    }

    /// <summary>通知总线切换器 RTMP 分支是否有真实消费者。</summary>
    public void SetRtmpConsumerAttached(bool attached)
    {
        lock (_gate)
        {
            _finalPcmBusTrackSwitch?.SetRtmpConsumerAttached(attached);
        }
    }

    /// <summary>
    /// 启动本机音频会话；真实路径必须来自已验证媒体资源清单和已探测媒体源。
    /// 传入最终 PCM 总线时，控制器在会话成功后接管其关闭生命周期。
    /// </summary>
    public async Task<WindowsAudioPlaybackResult> StartAsync(
        FfmpegPcmDecodePlan? plan,
        string? portAudioDllPath,
        WindowsPortAudioOutputConfig outputConfig,
        bool loop,
        CancellationToken cancellationToken = default,
        FinalPcmBus? finalPcmBus = null,
        Func<AudioPcmMixPolicy>? baseMixPolicyProvider = null,
        bool enableInterludeMix = false,
        AudioPcmMixEnvelopeOptions? mixEnvelopeOptions = null)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure("cancelled", "本机音频播放启动已取消。", retryable: true);
        }

        lock (_gate)
        {
            if (_disposed)
            {
                return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
            }
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
        }
        catch (OperationCanceledException)
        {
            return Failure("cancelled", "本机音频播放启动已取消。", retryable: true);
        }

        try
        {
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
                }

                if (_sessionTask is not null || _output is not null)
                {
                    return Reject("already_running", "本机音频播放已经在运行。", retryable: false);
                }

                _state = WindowsAudioPlaybackState.Starting;
                _lastError = null;
            }

            if (plan is null
                || outputConfig.Channels != plan.Channels
                || Math.Abs(outputConfig.SampleRate - plan.SampleRateHz) > 0.001
                || !double.IsFinite(plan.PlaybackRate) || plan.PlaybackRate is < 0.5 or > 2.0)
            {
                return Failure("invalid_plan", "本机音频播放计划、采样率或声道配置无效。", retryable: false);
            }

            if (finalPcmBus is not null
                && finalPcmBus.Channels != plan.Channels)
            {
                return Failure("invalid_final_pcm_bus", "最终 PCM 总线声道数与音频计划不一致。", retryable: false);
            }

            // 有限媒体项使用稳定的总线切换器；loop=true 保留既有单轨循环路径，
            // 因为它本身不跨候选边界。
            var ownsFinalPcmBus = !loop && finalPcmBus is null;
            var sessionFinalPcmBus = !loop
                ? finalPcmBus ?? new FinalPcmBus(_capacityFrames, plan.Channels)
                : finalPcmBus;
            plan = RemoveRealtimeInputThrottle(plan, hasBackpressure: sessionFinalPcmBus is not null);
            var finalPcmBusTrackSwitch = !loop && sessionFinalPcmBus is not null
                ? new FinalPcmBusTrackSwitch(activeCandidateId: 1, sessionFinalPcmBus)
                : null;
            var buffer = sessionFinalPcmBus?.OutputBuffer ?? new AudioPcmRingBuffer(_capacityFrames, plan.Channels);
            AudioPcmRingBuffer? overlayBuffer = null;
            AudioPcmMixingOutputSource? mixingOutputSource = null;
            if (enableInterludeMix)
            {
                overlayBuffer = sessionFinalPcmBus?.OutputOverlayBuffer
                    ?? new AudioPcmRingBuffer(_capacityFrames, plan.Channels);
                mixingOutputSource = finalPcmBusTrackSwitch is null
                    ? new AudioPcmMixingOutputSource(
                        buffer,
                        overlayBuffer,
                        plan.Channels,
                        policyProvider: baseMixPolicyProvider,
                        envelopeOptions: mixEnvelopeOptions)
                    : new AudioPcmMixingOutputSource(
                        finalPcmBusTrackSwitch.OutputSource,
                        finalPcmBusTrackSwitch.OutputOverlaySource,
                        plan.Channels,
                        policyProvider: baseMixPolicyProvider,
                        envelopeOptions: mixEnvelopeOptions);
            }

            var output = mixingOutputSource is null
                ? new WindowsPortAudioOutputStream(finalPcmBusTrackSwitch?.OutputSource ?? new AudioPcmRingBufferOutputSource(buffer))
                : new WindowsPortAudioOutputStream(mixingOutputSource);
            var decoder = new WindowsFfmpegPcmDecoder();
            var outputResult = await output.StartAsync(portAudioDllPath, outputConfig, cancellationToken).ConfigureAwait(false);
            if (!outputResult.IsSuccess)
            {
                if (output.HasPendingCleanup)
                {
                    lock (_gate)
                    {
                        _output = output;
                    }
                }
                else
                {
                    output.Dispose();
                }
                await decoder.DisposeAsync().ConfigureAwait(false);
                if (ownsFinalPcmBus)
                {
                    sessionFinalPcmBus?.Dispose();
                }
                return Failure(
                    outputResult.Error?.Code.ToString() ?? "output_start_failed",
                    outputResult.Error?.Message ?? "本机音频输出流启动失败。",
                    outputResult.Error?.Retryable == true);
            }

            var sessionCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            var pauseGate = new WindowsAudioPauseGate();
            Task sessionTask;
            lock (_gate)
            {
                _buffer = buffer;
                _output = output;
                _audibleAudioClock = new WindowsAudibleAudioClock();
                _audibleAudioClock.Anchor(
                    outputResult.Snapshot.MediaFramesWritten,
                    sourcePositionMs: plan.SourceStartMs,
                    playbackRate: plan.PlaybackRate);
                _decoder = decoder;
                _activePlan = plan;
                _activePlanCandidateId = 1;
                _finalPcmBus = sessionFinalPcmBus;
                _finalPcmBusTrackSwitch = finalPcmBusTrackSwitch;
                _pauseGate = pauseGate;
                _overlayBuffer = overlayBuffer;
                _mixingOutputSource = mixingOutputSource;
                // 混音输出源在 PortAudio 回调中统一应用基础轨/插话策略；若再让解码线程
                // 预先 duck，会造成基础轨被重复衰减。未启用混音时保留原有解码侧策略。
                _baseMixPolicyProvider = mixingOutputSource is null ? baseMixPolicyProvider : null;
                _sessionCancellation = sessionCancellation;
                _candidateSequence = 1;
                _candidateCompletion = !loop
                    ? CreateCandidateCompletion()
                    : null;
                _state = WindowsAudioPlaybackState.Playing;
                sessionTask = RunSessionAsync(
                    plan,
                    portAudioDllPath,
                    outputConfig,
                    loop,
                    sessionCancellation,
                    pauseGate);
                _sessionTask = sessionTask;
            }

            var ready = await WaitForInitialPcmAsync(
                    decoder,
                    output,
                    outputResult.Snapshot.MediaFramesWritten,
                    sessionTask,
                    sessionCancellation,
                    cancellationToken)
                .ConfigureAwait(false);
            return ready;
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>
    /// 启动唯一的 N+1 预载解码器。候选只写入自己的最终 PCM 总线，尚未提交前不影响当前输出。
    /// </summary>
    public async Task<WindowsAudioPlaybackResult> PrepareNextAsync(
        FfmpegPcmDecodePlan? plan,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Reject("candidate_cancelled", "下一音频候选预载已取消。", retryable: true);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
        }
        catch (OperationCanceledException)
        {
            return Reject("candidate_cancelled", "下一音频候选预载已取消。", retryable: true);
        }

        try
        {
            PreparedAudioCandidate candidate;
            WindowsAudioPauseGate pauseGate;
            FinalPcmBusTrackSwitch trackSwitch;
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
                }

                if (_finalPcmBusTrackSwitch is null
                    || _sessionTask is null
                    || _sessionCancellation is null
                    || _pauseGate is null
                    || _state is not (WindowsAudioPlaybackState.Playing or WindowsAudioPlaybackState.Paused))
                {
                    return Reject("candidate_not_running", "当前没有可预载的有限音频会话。", retryable: false);
                }

                if (_preparedCandidate is not null)
                {
                    return Reject("candidate_already_prepared", "已经存在一个待提交的下一音频候选。", retryable: false);
                }

                if (plan is null || plan.Channels != _finalPcmBusTrackSwitch.Channels
                    || !double.IsFinite(plan.PlaybackRate) || plan.PlaybackRate is < 0.5 or > 2.0)
                {
                    return Reject("candidate_invalid_plan", "下一音频候选计划或声道数无效。", retryable: false);
                }

                if (_finalPcmBusTrackSwitch.RtmpConsumerAttached && Math.Abs(plan.PlaybackRate - 1.0) > 0.001)
                {
                    return Reject("rtmp_speed_unsupported", "活动推流不接受改变源时长的声音候选。", retryable: false);
                }

                if (_candidateSequence == ulong.MaxValue)
                {
                    return Reject("candidate_id_exhausted", "音频候选身份已达到上限。", retryable: false);
                }

                trackSwitch = _finalPcmBusTrackSwitch;
                pauseGate = _pauseGate;
                var candidateId = ++_candidateSequence;
                var bus = new FinalPcmBus(_capacityFrames, plan.Channels);
                if (!trackSwitch.TryPrepareNext(candidateId, bus, out var switchError))
                {
                    bus.Dispose();
                    return Reject(
                        "candidate_prepare_failed",
                        switchError?.Message ?? "下一音频候选登记失败。",
                        retryable: true);
                }

                candidate = new PreparedAudioCandidate
                {
                    CandidateId = candidateId,
                    Plan = RemoveRealtimeInputThrottle(plan),
                    Bus = bus,
                    Decoder = new WindowsFfmpegPcmDecoder(),
                    Cancellation = CancellationTokenSource.CreateLinkedTokenSource(
                        _sessionCancellation.Token,
                        cancellationToken),
                    Completion = CreateCandidateCompletion(),
                    CommitRequested = CreateBooleanCompletion(),
                    Activated = CreateBooleanCompletion(),
                };
                _preparedCandidate = candidate;
                candidate.DecodeTask = RunPreparedCandidateAsync(candidate, pauseGate);
            }

            return Success();
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>取消尚未提交的 N+1 候选，供暂停和配置切换丢弃过期预载。</summary>
    public async Task<WindowsAudioPlaybackResult> CancelPreparedNextAsync(
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Reject("candidate_cancelled", "下一音频候选取消已取消。", retryable: true);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
        }
        catch (OperationCanceledException)
        {
            return Reject("candidate_cancelled", "下一音频候选取消已取消。", retryable: true);
        }

        try
        {
            PreparedAudioCandidate? candidate;
            lock (_gate)
            {
                candidate = _preparedCandidate;
            }

            if (candidate is null)
            {
                return Success();
            }

            if (candidate.CommitRequested.Task.IsCompletedSuccessfully)
            {
                return Reject("candidate_commit_in_progress", "下一音频候选已提交，等待当前边界完成。", retryable: true);
            }

            await CancelPreparedCandidateAsync(candidate).ConfigureAwait(false);
            return Success();
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>
    /// 提交当前唯一的 N+1 候选；方法在本机输出真正切换后才返回，失败不伪造已切换。
    /// </summary>
    public async Task<WindowsAudioPlaybackResult> CommitPreparedNextAsync(
        CancellationToken cancellationToken = default,
        ulong? framesFromNow = null,
        ulong? targetPositionMs = null)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Reject("candidate_cancelled", "下一音频候选提交已取消。", retryable: true);
        }

        PreparedAudioCandidate? candidate;
        FinalPcmBusTrackSwitch? trackSwitch;
        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
        }
        catch (OperationCanceledException)
        {
            return Reject("candidate_cancelled", "下一音频候选提交已取消。", retryable: true);
        }

        try
        {
            lock (_gate)
            {
                candidate = _preparedCandidate;
                trackSwitch = _finalPcmBusTrackSwitch;
                if (candidate is null || trackSwitch is null)
                {
                    return Reject("candidate_not_prepared", "没有已预载的下一音频候选。", retryable: false);
                }

                if (_state is not (WindowsAudioPlaybackState.Playing or WindowsAudioPlaybackState.Paused))
                {
                    return Reject("candidate_not_running", "当前音频会话不允许提交下一候选。", retryable: false);
                }
            }
        }
        finally
        {
            _lifecycle.Release();
        }

        if (candidate is null || trackSwitch is null)
        {
            return Reject("candidate_not_prepared", "没有已预载的下一音频候选。", retryable: false);
        }

        bool candidateReady;
        try
        {
            candidateReady = await WaitForCandidateReadyAsync(candidate, cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            if (!candidate.CommitRequested.Task.IsCompletedSuccessfully)
            {
                await CancelPreparedCandidateAsync(candidate).ConfigureAwait(false);
            }

            return Reject("candidate_cancelled", "下一音频候选提交已取消。", retryable: true);
        }

        if (!candidateReady)
        {
            await CancelPreparedCandidateAsync(candidate).ConfigureAwait(false);
            return Reject("candidate_not_ready", "下一音频候选未在预载预算内提供首段 PCM。", retryable: true);
        }

        if (targetPositionMs is ulong target)
        {
            framesFromNow = CalculateFramesUntilPosition(target);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
        }
        catch (OperationCanceledException)
        {
            return Reject("candidate_cancelled", "下一音频候选提交已取消。", retryable: true);
        }

        try
        {
            lock (_gate)
            {
                if (!ReferenceEquals(_preparedCandidate, candidate))
                {
                    return Reject("candidate_stale", "下一音频候选已经失效。", retryable: true);
                }

                var commitSucceeded = framesFromNow is ulong targetFrames
                    ? trackSwitch.TryCommitNextAtFrames(candidate.CandidateId, targetFrames, out var switchError)
                    : trackSwitch.TryCommitNext(candidate.CandidateId, out switchError);
                if (!commitSucceeded)
                {
                    return Reject(
                        "candidate_commit_failed",
                        switchError?.Message ?? "下一音频候选提交失败。",
                        retryable: true);
                }

                candidate.CommitRequested.TrySetResult(true);
            }
        }
        finally
        {
            _lifecycle.Release();
        }

        try
        {
            var activated = await candidate.Activated.Task
                .WaitAsync(CandidateTransitionTimeout, cancellationToken)
                .ConfigureAwait(false);
            return activated
                ? Success()
                : Reject("candidate_activation_failed", "下一音频候选未完成输出切换。", retryable: true);
        }
        catch (TimeoutException)
        {
            return Reject("candidate_activation_timeout", "下一音频候选未能在切换预算内生效。", retryable: true);
        }
        catch (OperationCanceledException)
        {
            return Reject("candidate_cancelled", "下一音频候选提交已取消。", retryable: true);
        }
    }

    /// <summary>
    /// 在已启用混音的当前主音频会话上启动一次有界插话解码。
    /// 插话结束后不会自动续播，也不会创建第二个 PortAudio 输出流。
    /// </summary>
    public async Task<WindowsAudioPlaybackResult> StartInterludeAsync(
        FfmpegPcmDecodePlan? plan,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Failure("cancelled", "插话解码启动已取消。", retryable: true);
        }

        try
        {
            await _lifecycle.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
        }
        catch (OperationCanceledException)
        {
            return Failure("cancelled", "插话解码启动已取消。", retryable: true);
        }

        try
        {
            WindowsAudioPlaybackState state;
            WindowsAudioPauseGate? pauseGate;
            CancellationTokenSource? sessionCancellation;
            CancellationToken sessionToken;
            AudioPcmRingBuffer? overlayBuffer;
            FinalPcmBus? finalPcmBus;
            lock (_gate)
            {
                state = _state;
                pauseGate = _pauseGate;
                sessionCancellation = _sessionCancellation;
                sessionToken = sessionCancellation?.Token ?? default;
                overlayBuffer = _overlayBuffer;
                finalPcmBus = _finalPcmBusTrackSwitch?.ActiveBus ?? _finalPcmBus;
                if (_mixingOutputSource is null)
                {
                    return Reject("interlude_mix_not_enabled", "当前音频会话未启用插话混音。", retryable: false);
                }

                if (_overlayTask is not null)
                {
                    return Reject("interlude_already_running", "插话解码已经在运行。", retryable: false);
                }
            }

            if (state is not (WindowsAudioPlaybackState.Playing or WindowsAudioPlaybackState.Paused)
                || pauseGate is null
                || sessionCancellation is null
                || overlayBuffer is null)
            {
                return Reject("not_running", "当前没有可承载插话的主音频会话。", retryable: false);
            }

            var channels = _buffer?.Snapshot.Channels ?? 0;
            if (plan is null || plan.Channels != channels)
            {
                return Reject("invalid_interlude_plan", "插话解码计划或声道数无效。", retryable: false);
            }

            var overlayCancellation = CancellationTokenSource.CreateLinkedTokenSource(
                sessionToken,
                cancellationToken);
            var decoder = new WindowsFfmpegPcmDecoder();
            var staleSession = false;
            lock (_gate)
            {
                if (_overlayTask is not null
                    || _sessionTask is null
                    || _state is not (WindowsAudioPlaybackState.Playing or WindowsAudioPlaybackState.Paused))
                {
                    staleSession = true;
                }
                else
                {
                    _overlayDecoder = decoder;
                    _overlayCancellation = overlayCancellation;
                    _overlayTask = RunInterludeAsync(
                        plan,
                        overlayBuffer,
                        finalPcmBus,
                        decoder,
                        pauseGate,
                        overlayCancellation);
                }
            }

            if (staleSession)
            {
                overlayCancellation.Dispose();
                await decoder.DisposeAsync().ConfigureAwait(false);
                return Reject("interlude_stale_session", "主音频会话已变化，插话启动被拒绝。", retryable: true);
            }

            return Success();
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>停止当前插话解码并在有界预算内 Join；主音频会话保持不变。</summary>
    public async Task<WindowsAudioPlaybackResult> StopInterludeAsync()
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

    /// <summary>暂停当前音频会话；保留环缓和解码位置，不丢弃未播放 PCM。</summary>
    public async Task<WindowsAudioPlaybackResult> PauseAsync()
    {
        try
        {
            await _lifecycle.WaitAsync().ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
        }

        try
        {
            WindowsAudioPauseGate? pauseGate;
            WindowsPortAudioOutputStream? output;
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
                }

                if (_sessionTask is null || _state != WindowsAudioPlaybackState.Playing)
                {
                    return Failure("not_running", "本机音频播放当前未处于可暂停状态。", retryable: false);
                }

                pauseGate = _pauseGate;
                output = _output;
            }

            if (pauseGate is null || output is null)
            {
                return Failure("invalid_session", "本机音频播放会话资源不可用。", retryable: false);
            }

            pauseGate.Pause();
            var outputResult = await output.PauseAsync().ConfigureAwait(false);
            if (!outputResult.IsSuccess)
            {
                return FailAndAbortSession(
                    pauseGate,
                    "pause_failed",
                    outputResult.Error?.Message ?? "本机音频输出暂停失败。",
                    retryable: true);
            }

            lock (_gate)
            {
                if (_state == WindowsAudioPlaybackState.Playing)
                {
                    _state = WindowsAudioPlaybackState.Paused;
                }
            }

            return Success();
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>恢复暂停的音频会话；恢复后继续消费原有环缓。</summary>
    public async Task<WindowsAudioPlaybackResult> ResumeAsync()
    {
        try
        {
            await _lifecycle.WaitAsync().ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
        }

        try
        {
            WindowsAudioPauseGate? pauseGate;
            WindowsPortAudioOutputStream? output;
            lock (_gate)
            {
                if (_disposed)
                {
                    return Failure("closed", "本机音频播放控制器已关闭。", retryable: false);
                }

                if (_sessionTask is null || _state != WindowsAudioPlaybackState.Paused)
                {
                    return Failure("not_paused", "本机音频播放当前未处于暂停状态。", retryable: false);
                }

                pauseGate = _pauseGate;
                output = _output;
            }

            if (pauseGate is null || output is null)
            {
                return Failure("invalid_session", "本机音频播放会话资源不可用。", retryable: false);
            }

            var outputResult = await output.ResumeAsync().ConfigureAwait(false);
            if (!outputResult.IsSuccess)
            {
                return FailAndAbortSession(
                    pauseGate,
                    "resume_failed",
                    outputResult.Error?.Message ?? "本机音频输出恢复失败。",
                    retryable: true);
            }

            pauseGate.Resume();
            lock (_gate)
            {
                if (_state == WindowsAudioPlaybackState.Paused)
                {
                    _state = WindowsAudioPlaybackState.Playing;
                }
            }

            return Success();
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    /// <summary>请求取消当前会话，并在有界预算内 Join 解码与输出资源。</summary>
    public async Task<WindowsAudioPlaybackResult> StopAsync()
    {
        try
        {
            if (!await _lifecycle.WaitAsync(StopTimeout).ConfigureAwait(false))
            {
                return Failure("stop_timeout", "本机音频生命周期仍在执行，保留资源供重试。", retryable: true);
            }
        }
        catch (ObjectDisposedException)
        {
            return Success();
        }

        try
        {
            Task? session;
            WindowsFfmpegPcmDecoder? decoder;
            WindowsPortAudioOutputStream? output;
            WindowsAudioPauseGate? pauseGate;
            lock (_gate)
            {
                if (_disposed && _sessionTask is null && _output is null)
                {
                    _state = WindowsAudioPlaybackState.Closed;
                    return Success();
                }

                _state = _disposed ? WindowsAudioPlaybackState.Closed : WindowsAudioPlaybackState.Stopping;
                pauseGate = _pauseGate;
                _sessionCancellation?.Cancel();
                session = _sessionTask;
                decoder = _decoder;
                output = _output;
            }

            pauseGate?.Close();
            decoder?.Stop();
            if (session is null)
            {
                if (await StopOwnedOutputAsync(output).ConfigureAwait(false) is string outputError)
                {
                    return Failure("output_stop_pending", outputError, retryable: true);
                }
                lock (_gate)
                {
                    _state = _disposed ? WindowsAudioPlaybackState.Closed : WindowsAudioPlaybackState.Idle;
                }

                return Success();
            }

            try
            {
                await session.WaitAsync(StopTimeout).ConfigureAwait(false);
            }
            catch (TimeoutException)
            {
                return Failure("stop_timeout", "本机音频播放未能在停止预算内结束。", retryable: true);
            }

            if (await StopOwnedOutputAsync(output).ConfigureAwait(false) is string stopError)
            {
                return Failure("output_stop_pending", stopError, retryable: true);
            }
            return Success();
        }
        finally
        {
            _lifecycle.Release();
        }
    }

    public ValueTask DisposeAsync()
    {
        lock (_gate)
        {
            _disposed = true;
            if (_disposeTask is null || _disposeTask.IsFaulted)
            {
                _disposeTask = DisposeCoreAsync();
            }
            return new(_disposeTask);
        }
    }

    private async Task DisposeCoreAsync()
    {
        var stopped = await StopAsync().ConfigureAwait(false);
        if (!stopped.IsSuccess)
        {
            throw new InvalidOperationException(stopped.Error?.Message ?? "本机声音资源尚未退出。");
        }
        GC.SuppressFinalize(this);
    }

    private async Task<string?> StopOwnedOutputAsync(WindowsPortAudioOutputStream? output)
    {
        if (output is null)
        {
            return null;
        }
        var stopped = await output.StopAsync().ConfigureAwait(false);
        if (!stopped.IsSuccess || output.HasPendingCleanup)
        {
            return stopped.Error?.Message ?? "PortAudio 原生资源仍在退出，保留所有者供重试。";
        }
        try
        {
            output.Dispose();
        }
        catch (Exception exception) when (exception is TimeoutException or InvalidOperationException)
        {
            return "PortAudio 原生资源仍在退出，保留所有者供重试。";
        }
        lock (_gate)
        {
            if (ReferenceEquals(_output, output))
            {
                _output = null;
            }
        }
        return null;
    }

    private async Task<WindowsAudioPlaybackResult> StopInterludeCoreAsync()
    {
        CancellationTokenSource? cancellation;
        WindowsFfmpegPcmDecoder? decoder;
        Task? task;
        lock (_gate)
        {
            cancellation = _overlayCancellation;
            decoder = _overlayDecoder;
            task = _overlayTask;
            cancellation?.Cancel();
        }

        if (cancellation is null || task is null)
        {
            return Success();
        }

        decoder?.Stop();
        try
        {
            await task.WaitAsync(StopTimeout).ConfigureAwait(false);
            return Success();
        }
        catch (TimeoutException)
        {
            return Failure("interlude_stop_timeout", "插话解码未能在停止预算内结束。", retryable: true);
        }
    }

    private async Task RunInterludeAsync(
        FfmpegPcmDecodePlan plan,
        AudioPcmRingBuffer overlayBuffer,
        FinalPcmBus? finalPcmBus,
        WindowsFfmpegPcmDecoder decoder,
        WindowsAudioPauseGate pauseGate,
        CancellationTokenSource cancellation)
    {
        try
        {
            WindowsFfmpegPcmDecoderResult result;
            if (finalPcmBus is null)
            {
                result = await decoder.DecodeAsync(
                        plan,
                        overlayBuffer,
                        cancellation.Token,
                        pauseGate.WaitIfPausedAsync)
                    .ConfigureAwait(false);
            }
            else
            {
                result = await decoder.DecodeAsync(
                        plan,
                        destination: null,
                        cancellation.Token,
                        pauseGate.WaitIfPausedAsync,
                        finalPcmBus,
                        baseMixPolicyProvider: null,
                        finalPcmOverlay: true,
                        finalPcmBusProvider: () => ActiveFinalPcmBus)
                    .ConfigureAwait(false);
            }
            if (!result.IsSuccess && !cancellation.IsCancellationRequested)
            {
                throw new InvalidOperationException(result.Error?.Message ?? "本机插话解码失败。");
            }
        }
        catch (OperationCanceledException) when (cancellation.IsCancellationRequested)
        {
        }
        finally
        {
            if (finalPcmBus is not null)
            {
                finalPcmBus.DiscardOverlayPending();
                var currentBus = ActiveFinalPcmBus;
                if (!ReferenceEquals(currentBus, finalPcmBus))
                {
                    currentBus?.DiscardOverlayPending();
                }
            }
            else
            {
                overlayBuffer.DiscardPending();
            }

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

    private async Task<WindowsFfmpegPcmDecoderResult> RunPreparedCandidateAsync(
        PreparedAudioCandidate candidate,
        WindowsAudioPauseGate pauseGate)
    {
        WindowsFfmpegPcmDecoderResult result;
        try
        {
            result = await candidate.Decoder.DecodeAsync(
                    candidate.Plan,
                    destination: null,
                    candidate.Cancellation.Token,
                    pauseGate.WaitIfPausedAsync,
                    candidate.Bus,
                    baseMixPolicyProvider: null,
                    finalPcmOverlay: false,
                    beforePublishWaiter: (frames, token) =>
                        WaitForBusCapacityAsync(candidate.Bus, frames, token))
                .ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (candidate.Cancellation.IsCancellationRequested)
        {
            result = new(
                false,
                candidate.Decoder.Snapshot,
                new WindowsFfmpegPcmDecoderError(
                    WindowsFfmpegPcmDecoderFailureCode.Cancelled,
                    "下一音频候选预载已取消。",
                    Retryable: true));
        }
        catch (Exception)
        {
            result = new(
                false,
                candidate.Decoder.Snapshot,
                new WindowsFfmpegPcmDecoderError(
                    WindowsFfmpegPcmDecoderFailureCode.ProcessFailed,
                    "下一音频候选预载失败。",
                    Retryable: true));
        }

        candidate.Bus.Close();
        candidate.Completion.TrySetResult(result);
        return result;
    }

    private async ValueTask WaitForBusCapacityAsync(
        FinalPcmBus bus,
        int frames,
        CancellationToken cancellationToken)
    {
        if (frames <= 0 || frames >= bus.CapacityFrames)
        {
            return;
        }

        while (true)
        {
            var snapshot = bus.Snapshot;
            if (snapshot.IsClosed)
            {
                return;
            }

            // RTMP 是独立的有界实时消费者，满载时由其环缓丢弃最旧帧；
            // 不能因为 RTMP 泵暂时未读而反向阻塞本机声音和视频时钟。
            var outputHasRoom = snapshot.OutputAvailableFrames <= snapshot.CapacityFrames - frames;
            if (outputHasRoom)
            {
                return;
            }

            await Task.Delay(CandidatePollInterval, cancellationToken).ConfigureAwait(false);
        }
    }

    private async Task<bool> WaitForCandidateReadyAsync(
        PreparedAudioCandidate candidate,
        CancellationToken cancellationToken)
    {
        var minimumFrames = Math.Max(
            1,
            Math.Min(
                candidate.Bus.CapacityFrames / 2,
                candidate.Plan.SampleRateHz * CandidateMinimumReadyMs / 1_000));
        var deadline = DateTime.UtcNow + CandidateReadyTimeout;
        while (DateTime.UtcNow < deadline
            && !cancellationToken.IsCancellationRequested
            && !candidate.Cancellation.IsCancellationRequested)
        {
            var snapshot = candidate.Bus.Snapshot;
            if (snapshot.OutputAvailableFrames >= minimumFrames)
            {
                return true;
            }

            if (candidate.DecodeTask?.IsCompleted == true)
            {
                return snapshot.OutputAvailableFrames > 0;
            }

            await Task.Delay(CandidatePollInterval, cancellationToken).ConfigureAwait(false);
        }

        return false;
    }

    private async Task CancelPreparedCandidateAsync(PreparedAudioCandidate candidate)
    {
        if (Interlocked.Exchange(ref candidate.CleanupStarted, 1) != 0)
        {
            return;
        }

        FinalPcmBusTrackSwitch? trackSwitch;
        lock (_gate)
        {
            if (ReferenceEquals(_preparedCandidate, candidate))
            {
                _preparedCandidate = null;
            }

            trackSwitch = _finalPcmBusTrackSwitch;
        }

        _ = trackSwitch?.TryCancelNext(candidate.CandidateId, out _);
        candidate.CommitRequested.TrySetResult(false);
        candidate.Activated.TrySetResult(false);
        candidate.Cancellation.Cancel();
        candidate.Decoder.Stop();
        if (candidate.DecodeTask is not null)
        {
            try
            {
                await candidate.DecodeTask.WaitAsync(StopTimeout).ConfigureAwait(false);
            }
            catch (Exception) when (candidate.Cancellation.IsCancellationRequested)
            {
            }
        }

        await candidate.Decoder.DisposeAsync().ConfigureAwait(false);
        candidate.Bus.Dispose();
        candidate.Cancellation.Dispose();
    }

    private static TaskCompletionSource<WindowsFfmpegPcmDecoderResult> CreateCandidateCompletion() =>
        new(TaskCreationOptions.RunContinuationsAsynchronously);

    private static TaskCompletionSource<bool> CreateBooleanCompletion() =>
        new(TaskCreationOptions.RunContinuationsAsynchronously);

    private ulong CalculateFramesUntilPosition(ulong targetPositionMs)
    {
        lock (_gate)
        {
            var clock = _audibleAudioClock?.Project(_output?.Snapshot);
            return CalculateFramesUntilPosition(clock, targetPositionMs);
        }
    }

    internal static ulong CalculateFramesUntilPosition(
        WindowsAudibleAudioClockSnapshot? clock, ulong targetPositionMs)
    {
        if (clock?.PlaybackTimeMs is not ulong currentPositionMs
            || targetPositionMs <= currentPositionMs
            || clock.SampleRateHz <= 0)
        {
            return 0;
        }

        var deltaMs = targetPositionMs - currentPositionMs;
        var playbackFrames = deltaMs * (double)clock.SampleRateHz / 1_000d / clock.PlaybackRate;
        var latencyFrames = (double)clock.OutputLatencyMicroseconds
            * clock.SampleRateHz
            / 1_000_000d;
        var frames = Math.Max(0, Math.Ceiling(playbackFrames) - Math.Ceiling(latencyFrames));
        return !double.IsFinite(frames) || frames >= ulong.MaxValue
            ? ulong.MaxValue
            : (ulong)frames;
    }

    internal static FfmpegPcmDecodePlan RemoveRealtimeInputThrottle(
        FfmpegPcmDecodePlan plan, bool hasBackpressure = true)
    {
        // 最终 PCM 总线按剩余容量等待；旧的独立环缓循环仍保留输入限速。
        if (!hasBackpressure)
        {
            return plan;
        }
        var realtimeIndex = plan.Arguments.IndexOf("-re");
        return realtimeIndex < 0
            ? plan
            : plan with { Arguments = plan.Arguments.RemoveAt(realtimeIndex) };
    }

    private async Task RunSessionAsync(
        FfmpegPcmDecodePlan plan,
        string? portAudioDllPath,
        WindowsPortAudioOutputConfig outputConfig,
        bool loop,
        CancellationTokenSource sessionCancellation,
        WindowsAudioPauseGate pauseGate)
    {
        var cancellationToken = sessionCancellation.Token;
        WindowsFfmpegPcmDecoder? decoder;
        FinalPcmBus? finalPcmBus;
        AudioPcmRingBuffer? buffer;
        WindowsPortAudioOutputStream? output;
        Func<AudioPcmMixPolicy>? baseMixPolicyProvider;
        FinalPcmBusTrackSwitch? trackSwitch;
        lock (_gate)
        {
            decoder = _decoder;
            finalPcmBus = _finalPcmBus;
            buffer = _buffer;
            output = _output;
            baseMixPolicyProvider = _baseMixPolicyProvider;
            trackSwitch = _finalPcmBusTrackSwitch;
        }

        if (decoder is null || buffer is null || output is null)
        {
            SetFailure("invalid_session", "本机音频播放会话资源不可用。", retryable: false);
            return;
        }

        var currentDecoder = decoder;
        var currentBus = finalPcmBus;
        PreparedAudioCandidate? activeCandidate = null;
        using var healthCancellation = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        var healthTask = ObserveOutputHealthAsync(
            output,
            portAudioDllPath,
            outputConfig,
            pauseGate,
            sessionCancellation,
            healthCancellation.Token);
        try
        {
            while (true)
            {
                WindowsFfmpegPcmDecoderResult result = default!;
                Task<WindowsFfmpegPcmDecoderResult> decodeTask;
                if (activeCandidate is null)
                {
                    var decodeBus = currentBus;
                    decodeTask = currentDecoder!.DecodeAsync(
                            plan,
                            decodeBus is null ? buffer : null,
                            cancellationToken,
                            pauseGate.WaitIfPausedAsync,
                            decodeBus,
                            baseMixPolicyProvider,
                            beforePublishWaiter: decodeBus is null
                                ? null
                                : (frames, token) => WaitForBusCapacityAsync(decodeBus, frames, token))
                        ;
                }
                else
                {
                    decodeTask = activeCandidate.DecodeTask!;
                }

                var promotedBeforeDecodeCompletion = false;
                while (true)
                {
                    if (decodeTask.IsCompleted)
                    {
                        result = await decodeTask.ConfigureAwait(false);
                        break;
                    }

                    PreparedAudioCandidate? scheduledCandidate;
                    lock (_gate)
                    {
                        scheduledCandidate = _preparedCandidate;
                    }

                    FinalPcmBus? retiredBusDuringDecode = null;
                    var promotedDuringDecode = scheduledCandidate is not null
                        && scheduledCandidate.CommitRequested.Task.IsCompletedSuccessfully
                        && trackSwitch is not null
                        && trackSwitch.TryAcknowledgePromotion(
                            scheduledCandidate.CandidateId,
                            out retiredBusDuringDecode,
                            out _);
                    if (promotedDuringDecode && scheduledCandidate is not null)
                    {
                        currentDecoder.Stop();
                        try
                        {
                            await decodeTask.WaitAsync(StopTimeout).ConfigureAwait(false);
                        }
                        catch (Exception) when (!cancellationToken.IsCancellationRequested)
                        {
                            // 周期切换会取消当前解码器；旧候选不再作为会话结果。
                        }

                        retiredBusDuringDecode?.Close();
                        await currentDecoder.DisposeAsync().ConfigureAwait(false);
                        if (activeCandidate is not null)
                        {
                            activeCandidate.Cancellation.Dispose();
                        }

                        activeCandidate = scheduledCandidate;
                        currentDecoder = scheduledCandidate.Decoder;
                        currentBus = scheduledCandidate.Bus;
                        lock (_gate)
                        {
                            if (ReferenceEquals(_preparedCandidate, scheduledCandidate))
                            {
                                _preparedCandidate = null;
                                _decoder = scheduledCandidate.Decoder;
                                _activePlan = scheduledCandidate.Plan;
                                _activePlanCandidateId = scheduledCandidate.CandidateId;
                                _finalPcmBus = trackSwitch!.ActiveBus;
                                _buffer = trackSwitch.ActiveBus.OutputBuffer;
                                _candidateCompletion = scheduledCandidate.Completion;
                            }
                        }

                        AnchorPromotedAudioClock(output, trackSwitch!, scheduledCandidate.Plan);
                        scheduledCandidate.Activated.TrySetResult(true);
                        promotedBeforeDecodeCompletion = true;
                        break;
                    }

                    await Task.Delay(CandidatePollInterval, cancellationToken).ConfigureAwait(false);
                }

                if (promotedBeforeDecodeCompletion)
                {
                    continue;
                }

                if (!result.IsSuccess)
                {
                    if (cancellationToken.IsCancellationRequested
                        && result.Error?.Code is (WindowsFfmpegPcmDecoderFailureCode.Cancelled
                            or WindowsFfmpegPcmDecoderFailureCode.Closed))
                    {
                        return;
                    }

                    SetFailure(result.Error?.Code.ToString() ?? "decode_failed", result.Error?.Message ?? "PCM 解码失败。", result.Error?.Retryable == true);
                    return;
                }

                if (loop || cancellationToken.IsCancellationRequested)
                {
                    if (cancellationToken.IsCancellationRequested)
                    {
                        return;
                    }

                    while (!cancellationToken.IsCancellationRequested
                        && buffer.Snapshot.AvailableFrames > _capacityFrames * 3 / 4)
                    {
                        await Task.Delay(TimeSpan.FromMilliseconds(10), cancellationToken).ConfigureAwait(false);
                    }

                    continue;
                }

                if (currentBus is null || trackSwitch is null)
                {
                    await DrainBufferAsync(buffer, cancellationToken).ConfigureAwait(false);
                    return;
                }

                currentBus.Close();
                if (activeCandidate is null)
                {
                    lock (_gate)
                    {
                        _candidateCompletion?.TrySetResult(result);
                    }
                }

                PreparedAudioCandidate? prepared;
                lock (_gate)
                {
                    prepared = _preparedCandidate;
                }

                if (prepared is null)
                {
                    await DrainBufferAsync(currentBus.OutputBuffer, cancellationToken).ConfigureAwait(false);
                    return;
                }

                bool commitRequested;
                try
                {
                    commitRequested = await prepared.CommitRequested.Task
                        .WaitAsync(CandidateTransitionTimeout, cancellationToken)
                        .ConfigureAwait(false);
                }
                catch (TimeoutException)
                {
                    SetFailure("candidate_commit_timeout", "下一音频候选未在提交预算内完成。", retryable: true);
                    return;
                }

                if (!commitRequested)
                {
                    return;
                }

                FinalPcmBus? retiredBus = null;
                var promoted = false;
                var promotionDeadline = DateTime.UtcNow + CandidateTransitionTimeout;
                while (!cancellationToken.IsCancellationRequested
                    && DateTime.UtcNow < promotionDeadline)
                {
                    if (trackSwitch.TryAcknowledgePromotion(
                            prepared.CandidateId,
                            out retiredBus,
                            out _))
                    {
                        promoted = true;
                        break;
                    }

                    await Task.Delay(CandidatePollInterval, cancellationToken).ConfigureAwait(false);
                }

                if (!promoted)
                {
                    SetFailure("candidate_activation_timeout", "下一音频候选未能在切换预算内生效。", retryable: true);
                    prepared.Activated.TrySetResult(false);
                    return;
                }

                retiredBus?.Close();
                lock (_gate)
                {
                    AnchorPromotedAudioClock(output, trackSwitch, prepared.Plan);
                }
                await currentDecoder.DisposeAsync().ConfigureAwait(false);
                if (activeCandidate is not null)
                {
                    activeCandidate.Cancellation.Dispose();
                }

                activeCandidate = prepared;
                currentDecoder = prepared.Decoder;
                currentBus = prepared.Bus;
                lock (_gate)
                {
                    if (ReferenceEquals(_preparedCandidate, prepared))
                    {
                        _preparedCandidate = null;
                        _decoder = prepared.Decoder;
                        _activePlan = prepared.Plan;
                        _activePlanCandidateId = prepared.CandidateId;
                        _finalPcmBus = trackSwitch.ActiveBus;
                        _buffer = trackSwitch.ActiveBus.OutputBuffer;
                        _candidateCompletion = prepared.Completion;
                    }
                }

                prepared.Activated.TrySetResult(true);
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        finally
        {
            healthCancellation.Cancel();
            try
            {
                await healthTask.WaitAsync(StopTimeout).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (healthCancellation.IsCancellationRequested)
            {
                // 正常结束时停止健康观察器。
            }
            catch (TimeoutException)
            {
                SetFailure("audio_health_timeout", "PortAudio 健康观察器未能在停止预算内结束。", retryable: true);
            }

            _ = await StopInterludeCoreAsync().ConfigureAwait(false);
            PreparedAudioCandidate? preparedCandidate;
            lock (_gate)
            {
                preparedCandidate = _preparedCandidate;
            }

            if (preparedCandidate is not null)
            {
                await CancelPreparedCandidateAsync(preparedCandidate).ConfigureAwait(false);
            }

            pauseGate.Close();
            if (trackSwitch is null)
            {
                if (finalPcmBus is null)
                {
                    buffer.Close();
                    _overlayBuffer?.Close();
                }
                else
                {
                    finalPcmBus.Close();
                }
            }
            else
            {
                trackSwitch.Dispose();
            }
            var outputStopError = await StopOwnedOutputAsync(output).ConfigureAwait(false);
            var outputStopped = outputStopError is null;
            if (!outputStopped)
            {
                SetFailure("output_stop_pending", outputStopError!, retryable: true);
            }
            await currentDecoder.DisposeAsync().ConfigureAwait(false);
            if (activeCandidate is not null)
            {
                activeCandidate.Cancellation.Dispose();
            }
            lock (_gate)
            {
                _sessionCancellation = null;
                _sessionTask = null;
                _decoder = null;
                _activePlan = null;
                _finalPcmBus = null;
                _finalPcmBusTrackSwitch = null;
                _pauseGate = null;
                _overlayBuffer = null;
                _mixingOutputSource = null;
                _baseMixPolicyProvider = null;
                _output = outputStopped ? null : output;
                _audibleAudioClock = null;
                _buffer = null;
                _preparedCandidate = null;
                _candidateCompletion = null;
                if (_state is not WindowsAudioPlaybackState.Failed)
                {
                    _state = _disposed
                        ? WindowsAudioPlaybackState.Closed
                        : cancellationToken.IsCancellationRequested
                            ? WindowsAudioPlaybackState.Idle
                            : WindowsAudioPlaybackState.Completed;
                }
            }

            sessionCancellation.Dispose();
        }
    }

    /// <summary>
    /// 启动结果必须证明 PortAudio 已消费首批主源 PCM；仅解码到环缓不算输出就绪。
    /// 这样主窗口不会在音频链实际失败或没有声音轨道时误显示“声音处理已应用”。
    /// </summary>
    private async Task<WindowsAudioPlaybackResult> WaitForInitialPcmAsync(
        WindowsFfmpegPcmDecoder decoder,
        WindowsPortAudioOutputStream output,
        ulong initialMediaFrames,
        Task sessionTask,
        CancellationTokenSource sessionCancellation,
        CancellationToken cancellationToken)
    {
        var deadline = DateTime.UtcNow + InitialPcmReadyTimeout;
        var decoderSnapshot = decoder.Snapshot;
        var outputSnapshot = output.Snapshot;
        try
        {
            while (outputSnapshot.MediaFramesWritten <= initialMediaFrames
                && !sessionTask.IsCompleted
                && DateTime.UtcNow < deadline)
            {
                await Task.Delay(CandidatePollInterval, cancellationToken).ConfigureAwait(false);
                var decoded = decoder.Snapshot;
                if (decoded.DecodedFrames > 0)
                {
                    decoderSnapshot = decoded;
                }
                outputSnapshot = output.Snapshot;
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            sessionCancellation.Cancel();
            decoder.Stop();
            await WaitForSessionStopAsync(sessionTask).ConfigureAwait(false);
            return Failure("cancelled", "本机音频播放启动已取消。", retryable: true);
        }

        if (outputSnapshot.MediaFramesWritten > initialMediaFrames)
        {
            // 循环播放可能在这里立刻开始下一轮解码并重置解码器的本轮计数；
            // 启动结果保留已经观测到的首帧快照，避免成功结果再次显示为 0 帧。
            // 不要求 DAC timeInfo；短音频可能已结束，但累计主源帧仍证明实际交给了输出。
            return new(true, Snapshot with { Decoder = decoderSnapshot, Output = outputSnapshot });
        }

        if (!sessionTask.IsCompleted)
        {
            SetFailure("audio_start_timeout", "本机音频在启动预算内没有向输出设备送出主源 PCM 帧。", retryable: true);
            sessionCancellation.Cancel();
            decoder.Stop();
        }

        await WaitForSessionStopAsync(sessionTask).ConfigureAwait(false);
        var snapshot = Snapshot;
        return Failure(
            snapshot.ErrorCode ?? "no_pcm_frames",
            snapshot.Error ?? "本机音频没有产生可播放的 PCM 音频帧。",
            retryable: true);
    }

    private async Task WaitForSessionStopAsync(Task sessionTask)
    {
        try
        {
            await sessionTask.WaitAsync(StopTimeout).ConfigureAwait(false);
        }
        catch (TimeoutException)
        {
            // StartAsync 仍需返回一个确定结果；会话的最终清理由其自身 finally 继续负责。
        }
        catch (OperationCanceledException)
        {
            // 会话取消属于启动失败的收尾路径。
        }
        catch (Exception)
        {
            SetFailure("audio_start_failed", "本机音频会话启动失败。", retryable: true);
        }
    }

    private async Task ObserveOutputHealthAsync(
        WindowsPortAudioOutputStream output,
        string? portAudioDllPath,
        WindowsPortAudioOutputConfig outputConfig,
        WindowsAudioPauseGate pauseGate,
        CancellationTokenSource sessionCancellation,
        CancellationToken cancellationToken)
    {
        var baselineCallbackCount = output.Snapshot.CallbackCount;
        var baselineXrunCount = output.Snapshot.XrunCount;
        var sustainedXrunRecoveries = 0;
        while (!cancellationToken.IsCancellationRequested)
        {
            await Task.Delay(TimeSpan.FromMilliseconds(250), cancellationToken).ConfigureAwait(false);
            var snapshot = output.Snapshot;
            if (pauseGate.IsPaused)
            {
                baselineCallbackCount = snapshot.CallbackCount;
                baselineXrunCount = snapshot.XrunCount;
                continue;
            }

            var sustainedXrun = ShouldRecoverFromSustainedXrun(
                snapshot,
                baselineCallbackCount,
                baselineXrunCount);
            var hardwareNeedsRecovery = snapshot.HardwareState is
                WindowsPortAudioHardwareState.Stopped
                or WindowsPortAudioHardwareState.Inactive
                or WindowsPortAudioHardwareState.QueryError;
            if (!hardwareNeedsRecovery && !sustainedXrun)
            {
                if (snapshot.XrunCount == baselineXrunCount)
                {
                    sustainedXrunRecoveries = 0;
                }

                continue;
            }

            if (sustainedXrun && sustainedXrunRecoveries >= MaxSustainedXrunRecoveries)
            {
                SetFailure(
                    "audio_output_overrun",
                    "PortAudio 输出持续欠载，已达到有界输出流重建上限。",
                    retryable: true);
                sessionCancellation.Cancel();
                return;
            }

            var recovered = await _outputRecovery.RecoverAsync(
                    output,
                    portAudioDllPath,
                    outputConfig,
                    cancellationToken)
                .ConfigureAwait(false);
            if (cancellationToken.IsCancellationRequested)
            {
                return;
            }

            if (recovered.IsSuccess)
            {
                sustainedXrunRecoveries = sustainedXrun
                    ? sustainedXrunRecoveries + 1
                    : 0;
                var newBaseline = output.Snapshot;
                baselineCallbackCount = newBaseline.CallbackCount;
                baselineXrunCount = newBaseline.XrunCount;
                continue;
            }

            SetFailure(
                "audio_device_lost",
                recovered.Error?.Message ?? "PortAudio 输出设备已断开，恢复失败。",
                retryable: true);
            sessionCancellation.Cancel();
            return;
        }
    }

    /// <summary>
    /// 只把持续性的 PortAudio xrun 视为需要重建；启动短暂欠载和一次设备回调抖动不触发重建。
    /// 基线由健康观察器在每次成功重建后更新，避免累计历史计数反复触发。
    /// </summary>
    internal static bool ShouldRecoverFromSustainedXrun(
        WindowsPortAudioOutputSnapshot snapshot,
        ulong baselineCallbackCount,
        ulong baselineXrunCount)
    {
        if (snapshot.CallbackCount < baselineCallbackCount
            || snapshot.XrunCount < baselineXrunCount)
        {
            return false;
        }

        var callbackDelta = snapshot.CallbackCount - baselineCallbackCount;
        var xrunDelta = snapshot.XrunCount - baselineXrunCount;
        if (callbackDelta < SustainedXrunMinimumCallbacks
            || xrunDelta < SustainedXrunMinimumEvents)
        {
            return false;
        }

        var sustainedEventMinimum = callbackDelta - callbackDelta / 4;
        return xrunDelta >= sustainedEventMinimum;
    }

    private static async Task DrainBufferAsync(
        AudioPcmRingBuffer buffer,
        CancellationToken cancellationToken)
    {
        var deadline = DateTime.UtcNow + DrainTimeout;
        while (!cancellationToken.IsCancellationRequested
            && buffer.Snapshot.AvailableFrames > 0
            && DateTime.UtcNow < deadline)
        {
            await Task.Delay(TimeSpan.FromMilliseconds(10), cancellationToken).ConfigureAwait(false);
        }
    }

    private void SetFailure(string code, string message, bool retryable)
    {
        lock (_gate)
        {
            _lastError = new(code, message, retryable);
            _state = WindowsAudioPlaybackState.Failed;
        }
    }

    private WindowsAudioPlaybackResult FailAndAbortSession(
        WindowsAudioPauseGate pauseGate,
        string code,
        string message,
        bool retryable)
    {
        var error = new WindowsAudioPlaybackError(code, message, retryable);
        CancellationTokenSource? cancellation;
        WindowsFfmpegPcmDecoder? decoder;
        lock (_gate)
        {
            _lastError = error;
            _state = WindowsAudioPlaybackState.Failed;
            cancellation = _sessionCancellation;
            decoder = _decoder;
            cancellation?.Cancel();
        }

        pauseGate.Close();
        decoder?.Stop();
        return new(false, Snapshot, error);
    }

    private WindowsAudioPlaybackResult Success() => new(true, Snapshot);

    /// <summary>
    /// 返回调用前置条件拒绝，但不把仍在运行的主会话污染成 Failed；
    /// 例如重复 Start、重复 Pause 或插话能力未启用都不应停止现有输出。
    /// </summary>
    private WindowsAudioPlaybackResult Reject(string code, string message, bool retryable)
    {
        var error = new WindowsAudioPlaybackError(code, message, retryable);
        lock (_gate)
        {
            return new(false, CreateSnapshotUnsafe(), error);
        }
    }

    private WindowsAudioPlaybackResult Failure(string code, string message, bool retryable)
    {
        var error = new WindowsAudioPlaybackError(code, message, retryable);
        lock (_gate)
        {
            _lastError = error;
            if (_state is not WindowsAudioPlaybackState.Closed)
            {
                _state = WindowsAudioPlaybackState.Failed;
            }

            return new(false, CreateSnapshotUnsafe(), error);
        }
    }

    private WindowsAudioPlaybackSnapshot CreateSnapshotUnsafe() =>
        new(
            _state,
            _output?.Snapshot,
            _decoder?.Snapshot,
            _lastError?.Code,
            _lastError?.Message)
        {
            AudibleClock = _audibleAudioClock?.Project(_output?.Snapshot),
        };
}
